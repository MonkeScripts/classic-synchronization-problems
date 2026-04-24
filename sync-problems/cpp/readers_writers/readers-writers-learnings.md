# Readers-Writers (C++): Mistakes & Learnings

A record of implementing the readers-writers KV-cache in C++20 — three flavors (`std::shared_mutex`, hand-rolled Lightswitch, and the no-starve turnstile) — the bugs I hit along the way, and the conceptual points each one forced me to internalise.

---

## The headline observation

The mistakes here split cleanly into **three distinct families**, and each one taught something you don't usually get from a working solution:

1. **Instrumentation placement bugs.** Most of my early errors weren't in the synchronization logic — they were about *where* to call `counters_.enter_read()` / `enter_write()`. The template uses `ActiveCounters` as a *runtime oracle* that asserts the formal invariant; my first instinct was to use its counters as solution state, which broke the abstraction in both directions.

2. **Single-op atomicity vs multi-op atomicity.** I hit the classic "I made it atomic so it must be thread-safe" trap on `rc_`. Making a variable `std::atomic<int>` fixes data races on individual ops; it doesn't make a *decrement-then-branch* sequence atomic. The mutex has to cover the entire decision.

3. **The fairness gap between textbook pseudocode and real primitives.** The no-starve turnstile implementation is correct on paper but turned out to be flaky in practice (~85% pass rate, timeouts on the rest). `std::counting_semaphore` doesn't guarantee FIFO wakeups, which is what Downey's "no-starve" proof implicitly assumes. This was the most useful discovery — the textbook works in the abstract model; real semaphores aren't it.

---

## The journey at a glance

| # | Mistake | Fix | Category |
|---|---|---|---|
| 1 | Called `counters_.enter_read()` **before** acquiring `roomEmptySem_`, with reasoning that "it's just an atomic `+1`, what could go wrong?" | Move `enter_read()` to AFTER the thread has actually secured exclusion from writers. See deep-dive below. | instrumentation placement |
| 2 | Used `counters_.readers == 1` as the "am I the first reader?" check, coupling solution state to the invariant oracle | Maintain a separate `int rc_` (or `std::atomic<int>`) as solution state; keep `counters_` strictly for instrumentation | separation of concerns |
| 3 | `int rc_;` declared without an initialiser — UB on first read | `int rc_ = 0;` or `{0};` | C++ fundamentals |
| 4 | `--rc_;` placed OUTSIDE `bibo_sem_` in the exit path, with `rc_` as a plain int | Put `--rc_` inside `bibo_sem_` so the decrement + check + conditional `roomEmptySem_.release()` is atomic as a group | compound-op race |
| 5 | "Fixed" #4 by changing `rc_` to `std::atomic<int>` instead of moving it into the lock | Atomic makes the individual `--rc_` atomic but doesn't make the sequence `decrement → check rc_==0 → conditionally release` atomic. Two threads can both observe `rc_==0` and both call `release()`, causing double-release (UB). Had to actually move the whole sequence inside `bibo_sem_`. | compound-op race, part 2 |
| 6 | `counters_.enter_read()` inside `bibo_sem_` (safe but over-serialized) | Hoisted it outside `bibo_sem_`; `counters_` members are atomic, so the invariant check runs just fine with `bibo_sem_` already released | performance |
| 7 | Implemented the no-starve turnstile variant following Downey's pseudocode verbatim, with `std::counting_semaphore<1>` for all three semaphores. Correct algorithmically, but intermittent deadlock (~15% of runs) | **Open — didn't fully resolve.** Evidence strongly suggests the spec of `std::counting_semaphore` doesn't guarantee FIFO wakeup, and one specific interleaving can livelock. See the deep-dive below. | fairness gap |

---

## The meta-lesson, front-loaded

> **The invariant counters (`ActiveCounters`) are a contract, not a state variable.** Every call to `enter_read()` / `enter_write()` says "I hereby claim that my delta does not violate the reader/writer invariant." If your synchronization hasn't yet ruled out the other side, calling `enter_*` is a false claim — and the instrumentation screams.

I tripped on this first (#1), then tried to route around it (#2), then had to accept that the separation was load-bearing. Much of the rest of the exercise was just following that consequence.

---

## Deep dive: why `counters_` and `rc_` must be separate

This is the single most important design choice in the template, and I pushed back on it for a while. For the record, the honest trade-off:

### What conflating them would buy

```cpp
// The "simpler" version I wanted
std::atomic<int> readers_{0}, writers_{0};

std::string get(...) {
    bibo_sem_.acquire();
    if (++readers_ == 1) roomEmptySem_.acquire();
    bibo_sem_.release();
    // read
    bibo_sem_.acquire();
    if (--readers_ == 0) roomEmptySem_.release();
    bibo_sem_.release();
}
```

15 lines, no `counters_` bookkeeping. Works, in the sense of "no data corruption happens."

### What it would lose

**1. The oracle is no longer independent of the algorithm.**
If your `readers_` counter IS your algorithm state AND your invariant witness, any bug that corrupts `readers_` is invisible to the thing meant to detect it. The whole value of `ActiveCounters` is that it's bumped at the *exact* moment a thread gains access to the shared data, decoupled from whatever bookkeeping the algorithm does internally.

**2. The three implementations stop being comparable.**
`KVCacheShared`, `KVCacheHandRolled`, and `KVCacheNoStarve` all wrap their critical sections with the same `counters_.enter_*` / `exit_*` calls. This gives a **uniform runtime invariant check across all three**. If each class had its own merged counter, each would need its own assertion logic, and the comparison across implementations would get murkier.

**3. The two quantities differ during the "first-reader-blocked" window.**
Concrete instant:
- Writer W is in the room. `roomEmptySem_ = 0`, W has already called `enter_write()` so `writers = 1`.
- Reader R1 arrives. R1 takes `bibo_sem_`, does `++rc_` (rc is now 1), blocks on `roomEmptySem_.acquire()`.

At this instant: *"threads that have entered the reader acquisition path" = 1*, but *"threads currently accessing the map" = 0*.

If `rc_` IS the invariant counter, the formal invariant "readers > 0 → writers == 0" is violated right now, even though no data race is happening. If `counters_.readers` is separate and is only bumped when R1 actually reaches the map, the invariant stays clean.

### The rule I took away

> If you're writing an algorithm and an invariant checker for the same piece of state, keep them as **two separate variables even though they conceptually track the same thing**, and have the invariant checker observe the state the algorithm produces rather than BE that state. The redundancy is what lets the checker catch bugs in the state machine.

In production this usually shows up as tracing probes, eBPF, or formal-verification assertions — all of them external witnesses over algorithmic state, never part of it.

---

## Deep dive: atomics vs the decrement-then-check race

This is the one I got wrong twice — first not seeing it, then thinking `std::atomic` had fixed it.

The exit path in the Lightswitch:
```cpp
--rc_;
if (rc_ == 0) roomEmptySem_.release();
```

If `rc_` is a plain `int`, the `--rc_` itself races when two readers exit simultaneously. Making it `std::atomic<int>` fixes *that* race.

But it doesn't fix this one:

```
T0: rc_ = 2
T1: Reader A: --rc_          // atomic: rc_ goes 2 → 1
T2: Reader B: --rc_          // atomic: rc_ goes 1 → 0
T3: Reader A: reads rc_       // sees 0
T4: Reader A: roomEmpty.release()   // first release
T5: Reader B: reads rc_       // also sees 0
T6: Reader B: roomEmpty.release()   // SECOND release — semaphore at max, UB
```

Each individual operation (the decrement, each read) is atomic. The *compound* sequence "decrement, observe, branch on observation" is NOT. You need a mutex around the whole block so the `--rc_` and the `if (rc_ == 0) ...release()` together are indivisible from any other thread's perspective:

```cpp
bibo_sem_.acquire();
--rc_;
if (rc_ == 0) roomEmptySem_.release();
bibo_sem_.release();
```

Now only one reader at a time can execute the decrement-and-check, so only one can observe `rc_ == 0`, so exactly one `release()` happens. And because the block is serialised, `rc_` doesn't even need to be atomic anymore — plain `int` works.

### The rule I took away

> `std::atomic` gives you **per-operation** atomicity. Mutexes give you **multi-operation** atomicity across the critical section. If your logic spans more than one access (decrement-then-check, read-then-update, compare-then-store-without-compare_exchange), you need a mutex around the *entire* sequence, or a proper atomic RMW like `compare_exchange_weak`. The per-op atomicity is a trap: it makes the code *look* thread-safe while the higher-level pattern still races.

This is the single most transferable lesson from this exercise. It comes up constantly.

---

## Deep dive: why the Lightswitch starves writers

Once the Lightswitch is correct, the feedback-checklist for this problem asks: *can writers starve?* The answer is yes, by design, and it's worth being able to say exactly why.

The Lightswitch reader path:
```
bibo.acquire
++rc
if rc == 1: roomEmpty.acquire     // first reader locks out writers
bibo.release
... read ...
bibo.acquire
--rc
if rc == 0: roomEmpty.release     // last reader releases
bibo.release
```

Under steady reader load:
- Reader R1 arrives, `rc = 1`, takes `roomEmpty`.
- Reader R2 arrives, `rc = 2`, skips (not first).
- R1 finishes, `rc = 1`, skips (not last).
- Reader R3 arrives, `rc = 2`, skips.
- ... and so on ...

**`rc` never returns to 0** as long as new readers keep arriving faster than existing ones exit. `roomEmpty` is held continuously by the readers collective. A writer arriving in this storm sits on `roomEmpty.acquire()` indefinitely — not because of a bug, but because the algorithm genuinely doesn't look at the writer queue. Readers don't know they should step aside.

This is intentional in the textbook — the Lightswitch is explicitly labelled *reader-preference*. The point is: correctness (no data race, invariant holds) and liveness (writers eventually proceed) are **separate concerns**. The invariant checker passes forever; starvation is a liveness bug the oracle cannot see.

### How to measure it

The benchmark as-is doesn't expose the problem with `NUM_READERS=8, NUM_WRITERS=2`. To make the starvation visible, you want:
- Bigger reader:writer ratio (try 50:1).
- Longer read critical sections (insert a small `std::this_thread::sleep_for(1us)` to prevent the gap where `rc` hits 0).
- A per-write wait-time sample (capture `steady_clock::now()` at entry to `set()`, sample again right after `roomEmptySem_.acquire()` returns, take the max).

With that, you get a concrete number: max writer wait time grows without bound.

---

## Deep dive: the turnstile, and why mine was flaky

The no-starve variant (Downey slide 12, *The Little Book of Semaphores*) adds a third semaphore — the **turnstile** — that every thread must pass through before entering its acquisition path.

```
Writer:
    turnstile.wait()
    roomEmpty.wait()
    ...write...
    roomEmpty.signal()
    turnstile.signal()

Reader:
    turnstile.wait()
    turnstile.signal()   // gate-pass: take and release immediately
    bibo.wait()
    ++rc
    if rc == 1: roomEmpty.wait()
    bibo.signal()
    ...read...
    bibo.wait()
    --rc
    if rc == 0: roomEmpty.signal()
    bibo.signal()
```

**How this prevents writer starvation (in theory):**
- Writers hold `turnstile` from acquisition all the way through their critical section.
- While a writer is queued on `roomEmpty` (holding `turnstile`), no new reader can enter — readers block at `turnstile.wait()`.
- Existing readers finish, `rc` drains to 0, `roomEmpty` is released, the writer proceeds.
- Writer finishes, releases `turnstile`, queued readers gate through.

Logically airtight. My implementation was almost literally Downey's pseudocode, translated to `std::counting_semaphore<1>`.

### And yet

In practice:
- Cold single runs: 7 ms, works correctly.
- Stress loop: **~85% pass, 15% hang** (process never completes within a 3s timeout, threads stuck on futexes, no invariant aborts).
- Adding a `std::this_thread::sleep_for` monitor thread elsewhere in the benchmark sometimes unsticks the hang.
- Swapping `std::counting_semaphore<1>` for `std::mutex` on some (but not all) of the three semaphores reduced the hang rate without eliminating it.
- Rewriting in Downey's exact release order (`turnstile.signal()` before `roomEmpty.signal()`) didn't change the rate.

Diagnosis — the parts I'm confident about:

**1. It's not a bug in the algorithm.** No interleaving I traced reaches a state where every thread is waiting for a resource held by some other waiting thread. The deadlock graph, as modelled, has no cycle.

**2. The `std::counting_semaphore` spec does not guarantee FIFO or even "fair" wakeup.** When `release()` is called, *some* waiter is woken; which one is implementation-defined. libstdc++ on Linux uses futexes, which are not strictly FIFO — they're "fair enough" for most uses but not for an algorithm whose liveness proof depends on eventually-fair wakeup.

**3. The hot spot is the reader's gate-pass `turnstile.acquire() / turnstile.release()`.** Under reader-heavy load this primitive is hammered dozens of times per millisecond. The thundering herd of readers may keep waking each other before the thread blocking everyone else (the first reader stuck at `roomEmpty.acquire()` while holding `bibo_sem_`) gets a chance.

Diagnosis — the part I'm *not* confident about:

**4. The exact path to livelock.** I could not reproduce a specific interleaving that gets stuck with every thread permanently blocked. It feels more like pathological scheduling than a classical deadlock. Adding print tracing (which inserts latency) makes it pass; removing it makes it hang. That fingerprint — "only fails at full speed" — is how livelock presents.

### The honest conclusion

> **Downey's proof that the turnstile prevents writer starvation assumes fair scheduling.** That assumption is implicit in textbook pseudocode. `std::counting_semaphore` does not promise fair scheduling. In production C++ you'd either use a primitive that does (e.g., `std::condition_variable` with explicit FIFO queue state, or a fair-mutex library), or accept that the starve-free property is "best-effort under the scheduler's fairness in practice."

This gap — between the clean pseudocode promise and the reality of the underlying primitive — is the single most useful thing this exercise taught. Going in I would have said "this code is correct." Coming out I now say "this algorithm is correct under the assumption that `release()` wakes waiters fairly, which this runtime doesn't guarantee."

---

## Final results

| Implementation | Correctness (200 runs, plain release) | Correctness (TSan) | Notes |
|---|---|---|---|
| `KVCacheShared` (std::shared_mutex) | 200/200 ✓ | clean | ~3 ms |
| `KVCacheHandRolled` (Lightswitch) | 200/200 ✓ | clean | ~5 ms |
| `KVCacheNoStarve` (turnstile) | **~85% pass**, 15% timeout | not fully tested — flakiness masks any TSan signal | ~5 ms when it does complete |

The Lightswitch is a complete, shippable implementation. The no-starve variant passes when it passes but is *not* reliable enough to recommend using as-is. See the deep-dive above for why.

---

## Working `KVCacheHandRolled` (for future reference)

```cpp
std::string get(const std::string& k) {
    bibo_sem_.acquire();
    ++rc_;
    if (rc_ == 1) roomEmptySem_.acquire();
    bibo_sem_.release();
    counters_.enter_read();
    auto it = map_.find(k);
    std::string result = (it != map_.end()) ? it->second : "";
    counters_.exit_read();
    bibo_sem_.acquire();
    --rc_;
    if (rc_ == 0) roomEmptySem_.release();
    bibo_sem_.release();
    return result;
}

void set(const std::string& k, const std::string& v) {
    roomEmptySem_.acquire();
    counters_.enter_write();
    map_[k] = v;
    counters_.exit_write();
    roomEmptySem_.release();
}
```

Three properties worth naming:
- `counters_.enter_read()` is called **after** `bibo_sem_.release()`, i.e., after the reader has fully secured exclusion from writers. The invariant check is therefore always correct.
- `--rc_` is inside `bibo_sem_`, so the decrement-and-check-and-maybe-release block is atomic as a unit.
- `rc_` is a plain `int` (not `std::atomic`) because every access is under `bibo_sem_` — the mutex is doing the synchronization, the atomic would be belt-and-suspenders.

---

## Next moves

- [ ] **Expose the Lightswitch's writer starvation with numbers.** Add per-writer wait-time instrumentation, rerun at `NUM_READERS=50 / NUM_WRITERS=1 / OPS_PER_THREAD=5000`, and capture max wait-time as a concrete metric for the writeup.
- [ ] **Chase the no-starve flakiness.** Specifically:
  - Swap all three semaphores for `std::mutex` + `std::condition_variable` with explicit predicates. Verify whether the flakiness goes away.
  - If yes: the diagnosis ("`counting_semaphore` fairness gap") is confirmed.
  - If no: there's something else in the algorithm I'm missing.
- [ ] **Sabotage experiments** to cement the lessons:
  - Remove the `if (rc_ == 1)` guard on `roomEmptySem_.acquire()` so every reader tries it → predict which invariant fires and why.
  - Change `bibo_sem_` to not hold across the `roomEmptySem_.acquire()` call on the first-reader path → predict the race.
  - Put `counters_.enter_read()` back inside the `bibo_sem_.acquire()` / `release()` block → verify it's still correct, note the over-serialization cost.
- [ ] **Port to Rust / Go.** The Rust version can use `parking_lot::FairMutex` or similar to test the fairness hypothesis in a language where FIFO primitives are available. The Go version will probably use `sync.RWMutex` (which has explicit writer-preference since Go 1.x) as a natural baseline.
