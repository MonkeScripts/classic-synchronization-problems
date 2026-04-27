# Recap: 4 problems × 3 languages (Rust still 3-of-4 on DP)

Compiled from the per-problem learnings docs. Sources: `cpp/{producer_consumer,readers_writers,barrier,dining_philosophers}/*-learnings.md`, `rust/{...}/*-learnings.md`, `go/{...}/*-learnings.md`.

The point of this doc is to **resurface state**, not re-derive it. Each per-problem-per-language section is one screen; deeper detail lives in the originals.

---

## Done so far

| Problem | C++ | Rust | Go |
|---|---|---|---|
| Producer-consumer | condvar + semaphore variants | condvar + std-mpsc variants | channel variant |
| Readers-writers | RWMutex + Lightswitch + turnstile | RwLock + tokio turnstile | sync.RWMutex + Lightswitch+turnstile |
| Barrier | domino + preloaded + cond-gen | cond-gen + tokio preloaded turnstile | 2×WaitGroup swap + cond-gen |
| Dining philosophers | scoped_lock + footman + Tanenbaum-CV | — | odd/even ring + try-and-back-off |

Up next: **dining philosophers in Rust**, then barbershop.

---

## Per-problem × per-language summary

### Producer-consumer

**The core algorithm (English-first, language-free):**
- Bounded queue. Producers wait while full, consumers wait while empty.
- `push` waits *until* `space || closed` ; bails on `closed`.
- `pop` waits *until* `non-empty || closed` ; bails **only** on `closed && empty` (drain remaining items first).
- Predicate-loop, not `if` — handles spurious / stolen / broadcast-excess wakeups.
- After progress: notify the *opposite* condition variable.

**C++ — condvar variant.** `std::mutex` + 2× `std::condition_variable`. `wait(lk, pred)` overload buries the while loop. Asymmetric bail (closed alone for push; closed+empty for pop). `close()` flips a `bool` and `notify_all` on both CVs. Gotchas drilled: predicate inversion ("until" vs "while"), missing `[this]` capture, lambda capture rules, charging through after waking when closed.

**C++ — semaphore variant.** `std::counting_semaphore<>` `spaces_sem_` + `items_sem_` + plain mutex for the deque. Producers acquire space / release item; consumers acquire item / release space. Big lessons: (1) `closed_` must be `std::atomic<bool>` because it's read outside the mutex (mixed-mode = UB even on `bool`); (2) **semaphores have no broadcast** — `close()` must `release(n)` enough permits per side, where n = max waiters that side; (3) **self-healing release-on-bail** — when a waiter wakes, observes closed, returns the permit. Over-release is harmless; under-release deadlocks.

**Rust — condvar variant.** Almost a direct C++ port. `Mutex<BufferInner<T>>` with one `closed` flag + the `VecDeque<T>` inside it. Bug taxonomy was overwhelmingly *Rust syntax*, not algorithm: implicit moves (no `std::move`), `Ok(())` (unit), `if` without `else` evaluates to `()`, state-lives-behind-the-lock (`inner.closed`, never `self.closed`), `wait` returns the guard back via `LockResult`, `is_empty` not `empty`. Predicate convention: Rust `wait_while` is the *opposite polarity* of C++ `wait(lock, pred)` — `wait_while(g, |s| !pred)` ≡ `cv.wait(lock, [&]{ pred })`.

**Rust — std-mpsc variant.** `sync_channel(N)` for the bounded buffer. Two pitfalls dominated: **(A)** `drop(tx)` in main *immediately after spawning* — channel closes only when **all** senders drop, so main's original `tx` keeps consumers blocked forever. **(B)** `Receiver` is **not `Clone`** — to share across consumers, wrap in `Arc<Mutex<Receiver<T>>>`, and **scope the lock tightly** (`let item = { let rx = lock.lock().unwrap(); rx.recv() };`) or you serialize on lock acquisition, not just dequeue. NUM_CONSUMERS = 2 with std-mpsc is *effectively serial*; `crossbeam-channel` removes this since its `Receiver` is `Clone`.

**Go — channel variant.** A channel **is** a bounded buffer: `make(chan T, N)`. Send blocks if full, receive blocks if empty. `range ch` drains until close. The single non-trivial concern is **who closes**: a dedicated closer goroutine (`go func() { pwg.Wait(); close(items) }()`), never an individual producer (race), never twice (panic). `close(done)` on a `chan struct{}` is the substitute for `notify_all` — every blocked `<-done` returns immediately. Counters live outside the channel — primitive synchronizes, you observe.

**Big trade-off table (across all three condvar/sem/channel takes):**

| Aspect | Condvar | Semaphore | Channel |
|---|---|---|---|
| Steady state | verbose (predicate, while, recheck) | short (acquire/work/release) | shortest (`<-` / `<-`) |
| Shutdown | 1 line: `notify_all` | release N permits per side + self-healing | `close(ch)` (or close-on-WG-Wait) |
| Failure mode | hang (silent) | hang or starve (silent) | runtime panic (loud) |
| Spurious wakeups | yes — predicate loop | no | no |
| Where complexity lives | steady-state predicate | shutdown sizing | who-closes coordination |

---

### Readers-writers

**The three algorithms, in increasing sophistication:**
1. **Built-in RW lock.** `std::shared_mutex` / `tokio::sync::RwLock` / `sync.RWMutex`. Baseline; "just works."
2. **Lightswitch (Downey).** Hand-rolled. Two semaphores: `bibo` (binary mutex protecting `rc`) + `roomEmpty` (held by reader collective or writer). First reader acquires `roomEmpty`, last reader releases. **Reader-preference — starves writers.**
3. **No-starve turnstile.** Add a third semaphore `turnstile`. Writers hold it for the whole CS; readers gate-pass (`acquire`+immediate `release`). A queued writer freezes new readers behind the turnstile.

**The headline cross-language finding (the marquee lesson of the entire problem):**

| Language | Primitive | FIFO wakeup? | Turnstile stress |
|---|---|---|---|
| C++ | `std::counting_semaphore<1>` | **No** ("at least one" thread unblocks) | ~85% pass, 15% timeout |
| Go | `chan struct{}` cap 1 | **Yes** (spec: served in FIFO order) | 500/500 clean |
| Rust | `tokio::sync::Semaphore` | **Yes** (docs: FIFO) | 500/500 clean |

Same algorithm. Same benchmark. Only the primitive's wakeup-fairness guarantee differs. **Downey's "no-starve" proof silently assumes fair scheduling.** That assumption is satisfied in two of three primitives. The C++ flakiness *is* the algorithm meeting an unfair primitive.

> **Correctness transfers; liveness doesn't.** Liveness is a joint property of the algorithm and the primitive's scheduler.

**C++ specifics.** Three families of bugs:
1. **Instrumentation placement** — `counters_.enter_read()` belongs *after* you've actually secured exclusion, not before.
2. **Atomics ≠ thread-safe sequences** — `--rc_; if (rc_ == 0) sem.release();` races as a *compound op* even when `rc_` is `std::atomic<int>`. Two threads can both observe `rc == 0` and both `release()` (UB on `counting_semaphore`). **Mutex around the whole sequence**, or use `compare_exchange`.
3. **Counter independence** — `counters_` (invariant oracle) and `rc_` (algorithm state) must be separate. They differ during the "first-reader-blocked" window: `rc==1` but no thread is actually in the room yet.

**Rust specifics (tokio).** New idioms:
- **No `.release()` method.** Permits are RAII — `let _p = sem.acquire().await.unwrap()` releases on drop. LIFO scope drop = reverse-acquisition release for free.
- **`permit.forget()` + `add_permits(1)`** — the escape hatch when first-reader acquires and last-reader (a different task) releases. RAII can't bridge tasks; this dance does.
- **`!Send` `MutexGuard` across `.await` is a compile error.** Tight-scope std mutex guards in inner blocks. `drop(guard)` does NOT help — the analysis is *lexical*, not dataflow. Rule: `std::sync::Mutex` only when the CS never `.await`s.
- Compound-op race resurfaced: use `fetch_add`'s **return value** (`let new_rc = rc.fetch_add(1, SeqCst) + 1;`), not a separate read.

**Go specifics.** Channel-as-semaphore patterns:
- `make(chan struct{})` (cap 0) — **rendezvous, NOT a semaphore**. Spent the most debugging time here.
- `make(chan struct{}, 1)` — binary semaphore.
- `make(chan struct{}, N)` — counting semaphore.
- `close(ch)` on a `chan struct{}` — broadcast (replaces `notify_all`).
- Two binary-semaphore *patterns*: A (textbook: seeded with token, recv=acquire, send=release) vs **B (Go-idiomatic: empty, send=acquire, recv=release)**. Pattern B scales to cap-N without rewriting; that's why it's idiomatic.

---

### Barrier

**The shapes:**
- **Domino (slide 20–21).** Two binary turnstiles. N-th arriver opens entry, the rest "domino" through (each acquires + immediately releases for the next). Phase 2 mirrors. Critical line: drain leftover token before reuse, or round-2 races through.
- **Preloaded (slide 22).** Single counting semaphore, init 0. N-th arriver `release(N)`. Each thread `acquire()`s exactly once. Reusable for free — N in, N out, count returns to 0. Requires `counting_semaphore<N>` not `<1>`.
- **Cond + generation counter.** `Mutex` + `Condvar` + `uint64 generation`. Snapshot gen at entry, last arriver flips `gen++` and `notify_all`. Predicate-loop on `gen != snapshot` handles spurious wakeups. **In production this is what you'd write** — fewest moving parts, hardest to get subtly wrong.

> **Generation counter, not a boolean flag.** A `bool ready` can't tell round R from round R+1.

**C++ specifics.** Domino phase-1 must `turnstile2.acquire()` *inside the lock* before opening entry — closes the exit gate before anyone leaves. Phase-2 must `turnstile1.acquire()` to drain the leftover token, or round 2 starts with the entry gate already open.

**Rust specifics.** Two variants: `main.rs` (cond + generation) and `bin/tokio.rs` (preloaded turnstile via `tokio::sync::Semaphore`).
- **Cond version friction was all type-system-pushing-bugs-to-compile-time:**
  - `state.count` lives behind the lock guard; `self.state.count` doesn't compile.
  - `let mut count = state.count` makes a local `usize` copy (Copy trap) — increments don't reach shared field → N-thread deadlock.
  - `std::sync::Mutex` is **not reentrant**; `wait_while(self.state.lock()...)` while holding the guard hangs.
  - Predicate polarity: Rust `wait_while(g, |s| pred)` = "wait *while* pred"; C++ `wait(l, pred)` = "wait *until* pred". **Inverse**.
  - `let _ = lock_guard` triggers `let_underscore_lock` (immediate drop). Use `let _guard = ...;` (named) to defer drop to scope end.
- **Tokio version friction was about the permit RAII model:**
  - `.await` is **postfix syntax**, not a method call. `expr.await`, never `expr.await()`.
  - `acquire()` returns `Result` because the semaphore can be `close()`d. Even if you never close, `.unwrap()` is required.
  - **`permit.forget()` is essential** for slide-22's preloaded turnstile. Without it, every acquired permit auto-returns on drop → gate never empties → round 2 races. `.forget()` consumes the permit without running its destructor.

**Go specifics.** Two variants: WaitGroup-swap (slide-23 shape) and cond+generation. The WG variant taught the lesson:
- **`sync.WaitGroup` is a count-down latch, not a cyclic barrier.** Reusing one panics: *"WaitGroup is reused before previous Wait has returned."*
- Two ways out: (1) snapshot-and-swap pattern with `*sync.WaitGroup` (need pointers — `sync.WaitGroup` carries `noCopy`, can't be compared by value, can't be copied), or (2) drop WG entirely → cond+generation.
- Composite literals can't call methods (`wg.Add(N)` returns nothing) — needs a `freshWG(n)` constructor helper.

---

### Dining philosophers

**The classic failure modes:**
- **Deadlock**: lock-acquire-graph cycle. Naive "everyone takes left first" → `0→1→2→3→4→0` cycle → hangs within seconds.
- **Livelock**: no thread blocks, no thread progresses. Try-and-back-off in lockstep is the canonical example.
- **Starvation**: deadlock-free + livelock-free, but one thread is consistently late. Tanenbaum's classic version starves a hungry philosopher between two alternating eaters.

**The five strategies (across all done implementations):**
1. **Naive** (every philosopher takes left then right) — the deadlock you're supposed to *see* on purpose.
2. **Asymmetric / odd-even ring** — one or more philosophers reverse the order. Breaks the cycle structurally in the lock-acquire graph.
3. **`std::scoped_lock` / try-and-back-off** — library-level (C++) or hand-rolled (Go labeled-break + non-blocking-select). Tries to acquire all, releases and retries on failure. Deadlock-free; technically livelock-prone but rare.
4. **Footman semaphore** — cap the number of hungry philosophers at N−1. Pigeonhole: no full cycle can form. Deadlock-free at runtime, but the lock-acquire graph still has a cycle, so static analyzers (TSan) flag it.
5. **Tanenbaum state-tracking** — no chopstick locks at all. Per-philosopher state under one mutex; per-philosopher CV (or semaphore). Neighbors `safe_to_eat` you when they finish. Deadlock-free, **starvation-prone**.

**The marquee cross-language finding (TSan vs Go race):**

| Strategy | C++ TSan | Go `-race` | Why |
|---|---|---|---|
| `std::scoped_lock` | clean | n/a | try-and-back-off internal — TSan recognizes |
| Footman | **lock-order-inversion warning** | n/a | Cycle in graph; cap is *runtime* prevention, invisible to TSan |
| Asymmetric (odd/even ring) | n/a | clean | Cycle absent in graph |
| Try-and-back-off | n/a | clean | No edges of "hold-while-blocking" form (the non-blocking try doesn't add an edge) |

> **TSan can detect *structural* deadlock potential. It cannot reason about *quantitative* arguments like "at most N−1 → no full cycle possible."** Same lesson family as the readers-writers fairness gap: an analyzer is right about what it can see; what it can't see is the whole prevention argument.

**C++ specifics.** Three flavors implemented: `eat_scoped_lock`, `eat_footman`, `eat_tanenbaum`. Big lessons:
- **Locks are useless if not shared across threads.** Stack-local `std::array<std::mutex, N>` per call → each philosopher gets private mutexes → no mutual exclusion → invariant fires.
- **`std::mutex` has no RAII semantics by itself.** `lock()` requires a matching `unlock()` — plain `{}` braces don't release. Use `lock_guard`/`unique_lock`/`scoped_lock` if you want RAII.
- **Tanenbaum CV version: predicate-loop is essential.** When `safe_to_eat(pid)` succeeds inside `take_forks`, state is already EATING and the predicate sees true → wait doesn't block. Without the predicate (using bare `cv.wait(lock)`), you'd block on a notify that already fired.
- **`algo_states[]` is separate from the oracle `states[]`.** Same separation principle as readers-writers: the witness must be independent of the algorithm.
- **The `==` vs `=` typo** is a one-character bug that produces total deadlock. `-Wall -Wextra` would have caught it via `-Wunused-comparison`.

**Go specifics.** Two variants: odd/even ring (`eat`) and try-and-back-off (`eatTryBackoff`).
- **The chopstick-as-channel pattern** is `chan struct{}` cap 1, primed with one `struct{}{}` token. Recv = acquire, send = release (Pattern A from readers-writers).
- **Labeled break** (`acquire:` + `break acquire`) is how you escape a `for` loop from inside a nested `select`. A bare `break` only exits the select case, leaving the loop running.
- **Non-blocking select** (`select { case <-ch: ...; default: ... }`) is the idiom for "try; if not ready, fall through." The `default:` is what flips the select from blocking to non-blocking.
- **Try-and-back-off avoids contributing edges to the deadlock graph at all** — by not waiting (non-blocking try), you don't hold-while-blocking. That's why both Go strategies pass `-race` clean.
- **Compile errors that don't read like brace errors:** `chan struct {` with a newline → parser thinks you've started a struct *type definition*, complains about field syntax later. `} else {` must be on one line because Go's automatic semicolon insertion ends the `if` after the `}`.
- **Arrays vs slices vs `make`:** `make` is for slices/maps/channels — not arrays. An array `[N]T` is zero-initialized automatically; only its elements (e.g., each individual channel) need `make`.
- **`struct{}` is the type, `struct{}{}` is the value.** Decompose: `struct{}` (empty struct type) + `{}` (zero-field literal).

**Rust:** still pending.

---

## Common patterns across all three problems

### 1. The instrumentation oracle is independent of the algorithm

Every template uses `counters_` / `Counters` / `ActiveCounters` as a *runtime invariant witness* — separate from solution state.

> If you're writing an algorithm and an invariant checker for the same piece of state, keep them as **two separate variables** even though they conceptually track the same thing. The redundancy is what lets the checker catch bugs in the state machine.

### 2. The compound-op race (the #1 transferable bug from C++)

```
--rc_;
if (rc_ == 0) roomEmpty.release();
```

Atomic on `rc_` doesn't fix this. Two threads can both observe 0 and both release. Mutex must wrap the *whole* sequence; or use a real atomic RMW (`compare_exchange_weak`).

This **also surfaced in Rust** — fix is to use `fetch_add`'s return value, not a separate read of the atomic afterwards.

### 3. The asymmetric bail (push vs pop)

Closing a producer-consumer queue:
- `push` on closed → bail immediately. No point pushing.
- `pop` on closed → drain remaining items first; bail only when *closed AND empty*.

This came up in C++ condvar, C++ semaphore, Rust condvar. Get it wrong → silently lose items.

### 4. Predicate orientation

| Library | API | Wait while...? |
|---|---|---|
| C++ | `cv.wait(lock, pred)` | `pred` is **false** (waits *until* true) |
| Rust | `cv.wait_while(guard, pred)` | `pred` is **true** (waits *while* true) |
| Go | manually `for !pred { cv.Wait(...) }` | explicit |

Translating between languages? **Read the function name before the body.** Polarity flips between C++ and Rust.

### 5. Where shutdown complexity lives

- **Condvar:** one line — `notify_all` on every CV anyone could be blocked on.
- **Semaphore:** must know max-waiters per side, `release(n)`, and waiters that bail must self-heal (re-release).
- **Channel (Go):** `close()`, but only senders may close, only once, and you usually need a closer goroutine watching a `WaitGroup`.
- **Tokio Semaphore:** RAII auto-handles release, but cross-task lifetimes need `forget()` + `add_permits()`.

### 6. Sanitizer / race-detector ritual

| Lang | Always-on tool | Build flag |
|---|---|---|
| C++ | TSan | `-DSANITIZER=thread` (per CLAUDE.md) |
| Rust | TSan (nightly) | `RUSTFLAGS="-Z sanitizer=thread" cargo +nightly run` — but on recent nightlies need `-Z build-std --target x86_64-unknown-linux-gnu` (sanitized user code can't link unsanitized prebuilt std) |
| Go | race detector | `go run -race .` |

Bar for "done" = 1000+ stress runs + sanitizer clean, not "passed once."

### 7. Sabotage experiments are the cheapest learning

Every learnings doc ends with a "30-second sabotage" list. Each one tests an invariant by removing it and predicting the failure. Pattern: edit → predict → run → reflect → revert. Faster than reading; sticks deeper.

---

## The three biggest distilled lessons

### A. Correctness vs liveness — and the fairness gap

> Concurrency textbook proofs silently assume fair scheduling. Real primitives vary in whether they honor that. **A correctness proof is only as strong as its weakest assumption.**

Manifested concretely as: same turnstile algorithm, ~85% pass on `std::counting_semaphore` (no FIFO), 100% on Go channels (FIFO) and tokio Semaphore (FIFO).

### B. Per-op atomicity ≠ multi-op atomicity

> `std::atomic` (and `AtomicI32`, `atomic.Int32`) gives **per-operation** atomicity. Mutexes give **multi-operation** atomicity across the critical section. If your logic spans more than one access — decrement-then-check, read-then-update, compare-then-store-without-CAS — you need a mutex around the *entire* sequence.

The trap: per-op atomicity makes the code *look* thread-safe while the higher-level pattern still races.

### C. Primitive choice shapes *where* the complexity lives

| Primitive | Steady state | Shutdown |
|---|---|---|
| Condvar | predicate loop, retest after wake | one `notify_all` line |
| Semaphore | clean acquire/release | release-N-permits-per-side dance |
| Channel | trivial (`<-`/`<-`/`range`) | "who closes" coordination + can panic |

Choose the primitive that puts complexity in the part of the code you're best at reading. If you can't state the algorithm in English without naming primitives, you haven't understood it yet.

---

## Language-specific cheat sheets

### Go
- `chan struct{}` is the all-purpose sync primitive. Cap 0 = rendezvous, cap 1 = mutex/binary-sem, cap N = counting-sem, `close()` = broadcast.
- **Idiomatic binary-sem pattern**: empty-buffer, send-to-acquire, recv-to-release ("I put myself in, I take myself out"). Pattern A (token-as-resource) is the alternative when the channel logically *carries* the resource (chopstick-as-token).
- `for range ch` to drain; `select` only when multiplexing or with `<-time.After`/`ctx.Done()`/`default:`.
- **Non-blocking select**: `select { case <-ch: ...; default: ... }` — try once, fall through. Without `default:`, blocks until a case fires.
- **Labeled break**: `acquire: for { ... break acquire }` — escapes the labeled loop from inside a nested select. Bare `break` only exits the case.
- `go func() { ... }()` — both `()` matter. The first defines, the second runs.
- Counters live outside primitives — `produced.Add(1)` is your job.
- `sync.WaitGroup` is a single-use latch, not cyclic. For barriers: pointer + swap, or cond+generation.
- **Arrays vs slices vs `make`**: `make` is for slices/maps/channels — never arrays. Array `[N]T` is zero-init by default; only the elements need `make` (e.g., each individual channel).
- **`struct{}` is the type, `struct{}{}` is the value** (`struct{}` + `{}` empty literal). `chan struct{}` is the canonical zero-byte signal channel.
- **Method receivers**: `func (s *Shared) Foo()` — `s` is the local name, `*Shared` is the type. Pointer receiver for mutation; Go auto-takes address at call site (`s.Foo()` works on a value too).
- **Brace placement is grammar, not style.** `} else {` must be one line; `func foo() {` opening brace on same line as header. Automatic semicolon insertion.
- Always `go run -race .` during development.

### Rust
- `Arc<T>` = spatial sharing across threads; `Mutex<T>` = temporal exclusion. `Arc<Mutex<T>>` does both.
- `Arc<T>` only hands out `&T`. Mutate through interior mutability: `Mutex`, `RwLock`, `AtomicI32`, etc.
- **Per `thread::spawn`, `Arc::clone(&x)` *before* the closure** (or shadow with `let x = Arc::clone(&x);`).
- `MutexGuard` is `!Send` — never hold a `std::sync::MutexGuard` across `.await` (lexical, so `drop()` doesn't help — use inner `{ }` blocks).
- `.await` is postfix: `expr.await`, no parens.
- Lock-aware lints: `let _ = lock.lock()` drops immediately (deny `let_underscore_lock`); `let _guard = ...` defers to scope.
- Tokio Semaphore: no `.release()`, permits drop to release. `permit.forget()` + `add_permits(1)` for cross-task lifetimes.
- `wait_while` polarity is **opposite** to C++ `wait(lock, pred)`.
- `match item` consumes `item` — inside `Ok(s)`, `item` is gone, only `s` remains.

### C++
- `std::condition_variable::wait(lock, pred)` overload — predicate-loop form. Lambda needs `[this]` capture for member access.
- Predicate answers *"what condition, once true, lets me stop waiting?"* — not *"what makes me wait?"*. Has a progress disjunct AND a give-up disjunct (`|| closed_`).
- After wake: distinguish *which* disjunct was satisfied.
- `std::counting_semaphore`: `acquire`/`release`, no broadcast. Over-release is UB. **No FIFO guarantee.**
- For shutdown of semaphore-based code: `release(MAX_WAITERS_PER_SIDE)` and waiters that bail re-release (self-healing).
- Mixed-mode access on `bool` is UB. Use `std::atomic<bool>` if read outside the lock anywhere.
- Build with `-DSANITIZER=thread`; expect to find races plain testing won't.

---

## Pre-Rust-DP / pre-barbershop checklist (carry-over)

Before opening any new template:
1. Read the scenario in `docs/scenarios.md` and the feedback checklist's section in `docs/feedback-checklist.md`.
2. State the algorithm in English. Name the strategies before typing.
3. Classic failure modes to track per problem: deadlock (cycle), livelock (everyone defers, no progress), starvation (one thread consistently late).
4. Stress + sanitizer. 100+ runs minimum, with jitter, before declaring done.
5. Template invariant rules still apply: oracle state (`states[]` / `Counters`) is the witness, separate from solution state. Sleep/jitter is intentional — don't remove it to make a flaky run pass.
6. Per-language defaults: Go `-race`, Rust nightly TSan with `-Z build-std`, C++ `-DSANITIZER=thread`.

For the Rust dining-philosophers specifically, expect the friction to be:
- Compile-time push-back of mistakes (the Rust pattern from prior writeups — readers-writers, barrier).
- `Arc<Mutex<...>>` shape for shared chopsticks; `Arc::clone(&x)` per `thread::spawn`.
- Probably no need for `tokio` — sync threads suffice unless you want async for parity.

---

## Cross-references

- `cpp/producer_consumer/producer-consumer-learnings.md` — the foundational condvar + semaphore writeup; sets the predicate-loop / asymmetric-bail / `notify` mental model used everywhere downstream.
- `cpp/readers_writers/readers-writers-learnings.md` — first sighting of the fairness gap (~85% pass on `counting_semaphore`).
- `go/readers_writers/readers-writers-learnings.md` — first FIFO confirmation (channels).
- `rust/readers_writers/readers-writers-learnings.md` — second FIFO confirmation (tokio); closes the three-language argument.
- `cpp/barrier/barrier-learnings.md` — domino vs preloaded vs cond+generation trade-off.
- `go/barrier/barrier-learnings.md` — `sync.WaitGroup` is a latch, not a barrier.
- `rust/barrier/barrier-learnings.md` — the longest catalogue of Rust-syntax-pushing-bugs-to-compile-time gotchas; useful as a Rust-idiom refresher even outside barriers.
- `go/producer_consumer/producer-consumer-learnings.md` — channel = bounded buffer; close-coordination pattern.
- `rust/producer_consumer/producer-consumer-learnings.md` — Part 2's mpsc deep-dive on `Arc<Mutex<Receiver>>` and lock-scope is the canonical reference for "channel handles vs the underlying channel."
- `cpp/dining_philosophers/dining-philosophers-learnings.md` — three strategies (scoped_lock, footman, Tanenbaum CV). The TSan lock-order-inversion warning on the footman is the canonical "structural analyzer can't see runtime invariants" example. The integrated semaphore-Tanenbaum walkthrough is the cleanest explanation of "semaphores have memory; signals encode the condition rather than control flow."
- `go/dining_philosophers/dining-philosophers-learnings.md` — two strategies (odd/even ring, try-and-back-off). Both pass `-race` clean — neither has a cycle in the relevant graph (odd/even prevents structurally; try-backoff doesn't add hold-while-blocking edges). Section 6 of "Go syntax fundamentals" is the canonical reference for labeled break + non-blocking select.
