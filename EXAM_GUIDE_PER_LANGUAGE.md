# CS3211 L9 — Concurrency Patterns: Per-Language Exam Guide

> Companion to `EXAM_GUIDE_PER_PROBLEM.md`. Use the per-problem guide to recall the algorithm; use this guide to recall the syntactic surface, idioms, and language-specific gotchas.
>
> Each language section is structured the same way:
>
> 1. **Primitive cheat sheet** — what each primitive maps to.
> 2. **Steady-state idioms** — predicates, locks, channels.
> 3. **Shutdown idioms** — how to terminate cleanly.
> 4. **Pitfalls catalogue** — every gotcha across all five problems.
> 5. **Build/run/debug** — sanitizer commands, race detection.
> 6. **Decision matrix** — which primitive when.
>
> The unifying meta-rule: **primitive choice shapes WHERE the complexity lives.** Pick the one that puts complexity in the part of the code you're best at reading.

---

## Table of contents

1. [C++ patterns](#1-c-patterns)
2. [Go patterns](#2-go-patterns)
3. [Rust patterns](#3-rust-patterns)
4. [Cross-language decision matrix](#4-cross-language-decision-matrix)
5. [Universal anti-patterns](#5-universal-anti-patterns)

---

## 1. C++ patterns

### 1.1 Primitive cheat sheet

| Primitive | Header | Default-constructible? | What it gives you |
|---|---|---|---|
| `std::mutex` | `<mutex>` | yes | mutual exclusion. **No RAII** by itself. |
| `std::scoped_lock<Mutexes...>` | `<mutex>` | n/a | RAII for one OR multiple mutexes (variadic uses `std::lock` try-and-back-off — multi-lock without deadlock!). |
| `std::lock_guard<Mutex>` | `<mutex>` | n/a | RAII for ONE mutex. Simpler than `unique_lock`; can't unlock/relock. |
| `std::unique_lock<Mutex>` | `<mutex>` | n/a | RAII for one mutex; supports unlock/relock (needed for `cv.wait`). |
| `std::shared_mutex` | `<shared_mutex>` | yes | reader-writer lock. `lock_shared()` for readers, `lock()` for writer. Pair with `std::shared_lock` (RAII for read). |
| `std::condition_variable` | `<condition_variable>` | yes | parking + notification. Always pair with mutex + predicate. |
| `std::counting_semaphore<MAX>` | `<semaphore>` (C++20) | **no** — needs init value | counter with `acquire`/`release`. **No FIFO guarantee.** **`release(n)` requires `MAX >= n`.** |
| `std::binary_semaphore` | `<semaphore>` | **no** | alias for `counting_semaphore<1>`. **Has memory** — useful for signal-once, even if waiter not yet parked. |
| `std::barrier<>` | `<barrier>` (C++20) | **no** — needs count | reusable phase synchronization. |
| `std::atomic<T>` | `<atomic>` | yes | per-op atomicity. **Default ordering: `memory_order_seq_cst`.** |
| `std::this_thread::sleep_for` | `<thread>` + `<chrono>` | n/a | typed sleep. `1ms`, `500us`, `2s` literals. |

### 1.2 Steady-state idioms

#### A. Condvar with predicate overload (the canonical wait shape)

```cpp
std::unique_lock<std::mutex> lk{mut_};
not_full_.wait(lk, [this] {                  // [this] capture — member access in lambda
    return queue_.size() < capacity_ || closed_;     // *** progress disjunct OR give-up disjunct ***
});
if (closed_) return false;                    // distinguish WHICH disjunct after wake
```

**Predicate orientation:** `cv.wait(lock, pred)` waits **until** `pred` is true. The body answers *"once true, I can stop waiting"* — not *"makes me wait."*

**Two valid forms — bare wait + manual loop, vs predicate overload:**

```cpp
// Verbose form — manual loop, easier to get wrong
while (!(queue_.size() < capacity_ || closed_)) cv.wait(lk);

// Predicate overload — internal loop, idiomatic
cv.wait(lk, [this] { return queue_.size() < capacity_ || closed_; });
```

#### B. Semaphore P/V

```cpp
sem.acquire();           // P / wait — blocks until permit available
// critical section
sem.release();           // V / signal — emits one permit
sem.release(n);          // emits n permits (for shutdown broadcast)
```

**No `notify_all` exists for semaphores.** To wake N parked waiters at shutdown: `sem.release(n)` and rely on each waiter's self-healing release-on-bail.

#### C. Multi-lock via `std::scoped_lock`

```cpp
std::scoped_lock lk{mu1, mu2};      // acquires BOTH using std::lock (try-and-back-off)
                                    // — deadlock-safe even if order is "wrong"
```

This is what makes the dining-philosophers `eat_scoped_lock` strategy work without a footman cap.

#### D. RAII releases in LIFO order at scope exit

```cpp
{
    std::lock_guard l1{mu1};         // declared first
    std::lock_guard l2{mu2};         // declared second
    // ...
}   // l2 destructs first (releases mu2), then l1 (releases mu1)
```

Reverse-of-acquire = conventional release order, gotten for free.

### 1.3 Shutdown idioms

#### Condvar shutdown (one line)

```cpp
void close() {
    { std::scoped_lock lk(mut_); closed_ = true; }
    not_full_.notify_all();
    not_empty_.notify_all();
}
```

`notify_all` is **the** broadcast mechanism. Notify every cv anyone could be parked on.

#### Semaphore shutdown (release N permits per side + self-heal)

```cpp
constexpr int MAX_PRODUCERS_WAITERS = NUM_PRODUCERS;
constexpr int MAX_CONSUMERS_WAITERS = NUM_CONSUMERS;

void close() {
    closed_.store(true);
    spaces_sem_.release(MAX_PRODUCERS_WAITERS);    // wake every parked producer
    items_sem_.release(MAX_CONSUMERS_WAITERS);     // wake every parked consumer
}

// In push/pop, the bail path RE-RELEASES the permit (self-healing):
bool push(T item) {
    spaces_sem_.acquire();
    if (closed_) { spaces_sem_.release(); return false; }
    // ...
}
```

**Self-healing:** any waiter that wakes, observes closed, and bails must re-release the permit it consumed. Over-released permits get recycled; under-released permits deadlock.

#### Worker loop with shutdown signal — recheck AFTER the wake

```cpp
while (true) {
    customer_sem_.acquire();
    if (stop_.load()) break;            // *** RECHECK between wake and work ***
    // do work
}
```

Top-of-loop check has already passed for the current iteration. The wake-up could be "real work" or "exit now"; the recheck disambiguates.

### 1.4 Pitfalls catalogue (every C++ gotcha across all problems)

#### Memory model
1. **Mixed-mode access on `bool`/any type is UB.** Plain `bool closed_` written under mutex but read outside → use `std::atomic<bool>`. TSan catches it; testing won't.
2. **`std::atomic` gives per-op atomicity only.** Compound ops like `--rc; if (rc == 0) sem.release()` race even with atomic `rc`. Use mutex around the **whole sequence** or `compare_exchange_weak`.

#### Condvar
3. **Inverted predicate.** `wait(lk, [] { return closed_; })` blocks while predicate is false — i.e., on a healthy buffer, sleeps forever. Predicate answers *"once true, stop waiting"*.
4. **Empty lambda capture `[]`.** Inside a member function, `queue_` is `this->queue_`. Use `[this]` or `[&]`.
5. **After waking, must check WHICH disjunct.** Predicate has progress AND give-up disjuncts; don't charge ahead.
6. **`return false` from `optional<T>`.** Use `std::nullopt`.
7. **Mutate predicate state OUTSIDE the lock.** Lost wakeup — `cv.notify_all` has no memory; the unlock-and-park atomicity is only w.r.t. holders of the *same* mutex. Always mutate under the lock, even with atomic types.
8. **No `return` in lambda predicate.** `cv.wait` requires bool-convertible — bare expression statement returns `void`.

#### Mutex / locking
9. **`std::mutex::lock()` without matching `unlock()`.** Plain `{}` braces don't release. Use `lock_guard`/`unique_lock`/`scoped_lock`, or write manual `unlock()` (in reverse-of-acquire order).
10. **Single global `std::mutex` for "the resource."** Identifies the wrong resource. Per-chopstick mutex lets non-adjacent philosophers eat simultaneously. **Match one mutex to one logical resource.**
11. **Locks declared INSIDE a function.** Stack-local → each call gets private locks → no mutual exclusion. **Sync primitives only work as shared objects across threads.** Promote to namespace/class scope.

#### Semaphore
12. **`acquire()` where you meant `release()`.** Read aloud: "acquire" = wait, "release" = signal. `close()`'s job is signal → release.
13. **`std::counting_semaphore<1>` then `release(n)`.** UB if `n > 1`. Use `counting_semaphore<N>` (or higher).
14. **Single shared `N_enough` for both directions.** Sizing is per-direction. Collapsing with `max()` over-releases harmlessly; collapsing too small deadlocks.
15. **No re-release on the closed-bail path.** Permits leak; over-released permits get silently consumed.
16. **`std::counting_semaphore` has NO FIFO guarantee.** Spec: "at least one thread unblocks." For algorithms that rely on FIFO (turnstile), this leaks ~15% timeout under stress.

#### Barrier
17. **`std::barrier<> b;`** — no default constructor; needs count. Same as semaphore.
18. **Domino phase 2 missing the `t1.acquire()` drain.** Leftover token leaks into round R+1; barrier broken.
19. **`bool ready_` instead of generation counter.** Round-tagging confusion → one-lap-ahead bug.

#### Other
20. **Tanenbaum `==` for `=` (assignment vs comparison).** Comparison-as-statement; result discarded → total deadlock on first meal. `-Wall -Wextra -Wunused-comparison` catches it.
21. **`sleep(1)` (POSIX, 1 SECOND) instead of `std::this_thread::sleep_for(1ms)`.** Wrong type, wrong unit, wrong API.
22. **`std::queue::front()` on empty queue.** UB. Always check `empty()` first.

### 1.5 Build / run / debug

```bash
# Regular build
cmake -S . -B build && cmake --build build
./build/<problem>/<binary>

# ThreadSanitizer (use during debugging)
cmake -S . -B build-tsan -DSANITIZER=thread && cmake --build build-tsan
./build-tsan/<problem>/<binary>

# AddressSanitizer (mutually exclusive with TSan)
cmake -S . -B build-asan -DSANITIZER=address && cmake --build build-asan

# Stress loop — typical "is it really fixed?" test
for i in {1..1000}; do ./build-tsan/<problem>/<binary> || break; done

# GDB for deadlocks
gdb ./<binary>
(gdb) run
# Ctrl-C when it hangs
(gdb) thread apply all bt
# Look for threads stuck in pthread_cond_wait or pthread_mutex_lock
```

**Bar for done:** 1000 stress runs + sanitizer clean. Not "passed once."

### 1.6 Decision matrix — which primitive when

| Need | Reach for | Why |
|---|---|---|
| Mutex serialising one resource | `std::mutex` + `std::lock_guard` | Cheap; RAII on the guard |
| Multi-mutex acquire (deadlock-safe) | `std::scoped_lock{a, b}` | Variadic form does internal try-and-back-off |
| Wait-on-predicate | `mutex + condition_variable + cv.wait(lk, pred)` | Express the condition directly |
| Resource pool / rate limit (count-based) | `counting_semaphore` | Counts naturally |
| Reader-writer | `shared_mutex` (baseline); hand-rolled Lightswitch+turnstile (illustrative) | Built-in is heavily optimised |
| Phase synchronization | `barrier` (slide 22) or cond+generation | Cond+gen is production-shape |
| Signal-with-memory (signal might fire before wait registers) | `binary_semaphore` | Has memory, unlike cv |

---

## 2. Go patterns

### 2.1 Primitive cheat sheet

| Primitive | Package | What it gives you |
|---|---|---|
| `chan T` (unbuffered) | builtin | rendezvous: send blocks until recv, recv blocks until send |
| `chan T` (buffered cap N) | builtin | tiny bounded queue |
| `chan struct{}` (cap 1) | builtin | binary semaphore / mutex |
| `chan struct{}` (cap N) | builtin | counting semaphore |
| `close(ch)` on `chan struct{}` | builtin | broadcast — every recv returns immediately (zero value) |
| `sync.Mutex` | `sync` | mutual exclusion. No reentrant. |
| `sync.RWMutex` | `sync` | reader-writer lock with writer preference |
| `sync.WaitGroup` | `sync` | **single-use latch**, NOT a cyclic barrier |
| `sync.Cond` | `sync` | parking on a predicate. Pair with mutex. |
| `sync.Once` | `sync` | run-exactly-once initialization |
| `atomic.Int32` etc. | `sync/atomic` | per-op atomicity |
| `time.Sleep`, `time.After` | `time` | delays, timeouts |
| `select { ... }` | builtin | multiplex over channel ops |
| `select { case x: ...; default: }` | builtin | non-blocking try |
| `select { case x: ...; case <-time.After(d): }` | builtin | timeout |

### 2.2 The big idea: **`chan struct{}` is Go's all-purpose sync primitive**

| Shape | Semantics | Use case |
|---|---|---|
| `make(chan struct{})` (cap 0) | rendezvous between two goroutines | handshake |
| `make(chan struct{}, 1)` | binary semaphore | mutex (use Pattern B below) |
| `make(chan struct{}, N)` | counting semaphore | rate limit, capacity bound |
| `close(ch)` | broadcast wake to every blocked recv | done channel — replaces `notify_all` |

You don't switch primitives; you switch **how you use one primitive**. That's a very different mental model from C++ where mutex / cv / semaphore / future are five distinct types.

### 2.3 Two binary-semaphore patterns

| | Pattern A (textbook) | **Pattern B (Go-idiomatic)** |
|---|---|---|
| Initial state | seeded with one token: `ch <- struct{}{}` at init | empty |
| Acquire | `<-ch` (take the token) | `ch <- struct{}{}` (put self in; blocks if full) |
| Release | `ch <- struct{}{}` (put back) | `<-ch` (take self out) |
| Mental model | "here's a token; take it" | "I put myself in, I take myself out" |
| Scales to N? | no | **yes** — `make(chan struct{}, N)` works without rewriting |

Pattern B is the ubiquitous Go shape:
```go
sem := make(chan struct{}, 10)
sem <- struct{}{}                              // acquire
defer func() { <-sem }()                       // release
```

Pattern A is the natural fit when the channel logically *carries* the resource (e.g., chopstick-as-token in dining philosophers).

### 2.4 Steady-state idioms

#### A. Drain a channel until close

```go
for item := range ch {                         // ranges drain until closed
    process(item)
}
```

If `item` is unused: `for range ch { ... }` (Go errors on unused locals).

#### B. Multiplex with `select`

```go
select {
case x := <-ch1:                                // arrived from ch1
case ch2 <- y:                                  // sent on ch2
case <-time.After(5 * time.Second):             // 5s timeout
case <-ctx.Done():                              // cancellation
}
```

Without a `default:`, blocks until at least one case fires. Multiple ready cases → **pseudo-random** selection (NOT source order).

#### C. Non-blocking try with `default:`

```go
select {
case ch <- x:    // sent
default:         // would have blocked → fall through
}
```

`default:` flips `select` from blocking to non-blocking.

#### D. Long-running worker loop

```go
for {
    select {
    case <-shutdown:
        return                                  // exit arm — return is correct here
    case x := <-work:
        process(x)                              // work arm — NO return; fall through
    }
}
```

**Only `return` from arms that should exit.** Returning from a work arm exits the function — the worker never serves customer #2.

#### E. Labeled break (escape `for` from inside `select`)

```go
acquire:
for {
    <-leftCh
    select {
    case <-rightCh:
        break acquire                           // *** breaks the LOOP, not just the select ***
    default:
        leftCh <- struct{}{}                    // give back
    }
}
```

Bare `break` only exits the `select` case. Labeled `break <label>` escapes the whole labeled construct.

#### F. Done-channel for broadcast

```go
done := make(chan struct{})
// ...workers select on <-done...
close(done)                                    // wakes EVERY blocked receiver (substitute for notify_all)
```

`close(ch)` on a `chan struct{}` is the canonical Go broadcast-wake.

### 2.5 Shutdown idioms

#### Closer-goroutine pattern (canonical)

```go
items := make(chan string, BufferSize)
var pwg sync.WaitGroup

for p := 0; p < NumProducers; p++ {
    pwg.Add(1)
    go func() { defer pwg.Done(); /* produce */ }()
}

go func() { pwg.Wait(); close(items) }()        // dedicated closer goroutine

// consumers: for range items { ... }
```

**Only senders may close. Closing twice panics. Sending on a closed channel panics.** A dedicated goroutine waits on producers' WaitGroup and is the **only** site that calls `close`.

#### Worker shutdown via select

```go
for {
    select {
    case <-shutdown:
        return
    case x := <-work:
        process(x)
    }
}
```

#### Via `close(done)` for broadcast cancellation

```go
done := make(chan struct{})
// ... workers select on <-done in addition to their work channels ...
close(done)                                    // wakes everyone simultaneously
```

### 2.6 Pitfalls catalogue (every Go gotcha across all problems)

#### Channel semantics
1. **`make(chan struct{})` (cap 0) for binary semaphore.** **Cap 0 is RENDEZVOUS, not a semaphore.** Send blocks until paired recv. For a semaphore you need cap 1 (state lives in the buffer).
2. **`nil` channel.** `var ch chan T` (no `make`) — sends/recvs block FOREVER. `new(MyType)` doesn't run `make` on channel fields. Force callers through constructors.
3. **Channel buffer size ≠ logical occupancy.** Buffered channel holds "queued, not yet picked up." Total in shop = buffer + (1 if cut in progress).
4. **Closing twice or sending on closed.** Runtime panic, not silent breakage. Use a closer goroutine; close once.
5. **`select` without default — blocking; with default — non-blocking.** Reach for `select` only when multiplexing or with `default:`/`time.After`/`ctx.Done()`.

#### Syntax / grammar
6. **`chan struct {` with newline before `}`.** Parser starts reading a struct type definition. Keep `chan struct{}` on one line.
7. **`} \n else {`.** ASI inserts `;` after `}`, ending the `if`. `} else {` MUST be on one line.
8. **Composite literal field set with `:=`.** Use `:` (key-value), not `:=` (short variable declaration).
9. **Missing trailing comma** on the last line of a multiline composite literal. Required.
10. **`go func() {...}` without `()`.** `go EXPR` requires a CALL, not a definition. Always `go func(){...}()`.
11. **Unused locals are compile errors.** Use `_ = x` or drop the binding.
12. **`for item := range ch` with `item` unused.** Same rule. Drop to `for range ch`.
13. **`struct{}` is the type; `struct{}{}` is the value.** `struct{}{} = struct{} (type) + {} (empty literal)`.

#### `sync.WaitGroup`
14. **Reusing one WG across rounds.** Panic: *"WaitGroup is reused before previous Wait has returned."* WaitGroup is a count-down latch, NOT cyclic. For barriers: snapshot-and-swap with `*sync.WaitGroup`, or skip to cond+generation.
15. **Field as `sync.WaitGroup` (value).** Carries `noCopy` lint. Can't be copied or compared. Use `*sync.WaitGroup`.
16. **`wg.Add(N)` inside a struct literal.** `Add` returns nothing; literals take values. Use a constructor.

#### Method receivers
17. **Receiver type mismatch.** `func (s *Shared)` requires the type to be `Shared` (capital). `*shared` (lowercase) is undefined.
18. **Bare field access without receiver inside methods.** Use `c.x`, never `x`. Go has no implicit `this`.

#### Arrays vs slices
19. **`make([]T, 0, N)` assigned to `[N]T` field.** Type mismatch — slice vs array. Arrays are zero-init by Go; only inner channels need `make`.
20. **`make` on an array.** `make` is for slices/maps/channels — never arrays.

#### Worker patterns
21. **`return` from a work arm of a long-running select.** Exits the whole function; worker dies after one job.
22. **`for { select { ... } }` where every arm returns.** Loop never iterates. Drop the `for`.
23. **`case default:`.** No `case` keyword; bare `default:`.

#### Counters / accounting
24. **Forgetting `produced.Add(1)` after a send.** Channels synchronize, NOT observe. Counters live OUTSIDE the primitive.

### 2.7 Build / run / debug

```bash
# Always use -race during development
go run -race .
go test -race ./...
go build -race

# Stress loop
for i in {1..1000}; do go run -race . || break; done

# Built-in deadlock detector (full deadlock — all goroutines blocked)
# Go's runtime PANICS automatically with "fatal error: all goroutines are asleep - deadlock!"
# This is huge. C++ has nothing like this.

# pprof for stuck goroutines (PARTIAL deadlock — some goroutines still running)
import _ "net/http/pprof"
go http.ListenAndServe("localhost:6060", nil)
# curl localhost:6060/debug/pprof/goroutine?debug=2

# SIGQUIT dumps all goroutine stacks
# Ctrl-\ on a hung program

# goleak for goroutine-leak detection
import "go.uber.org/goleak"
func TestMain(m *testing.M) { goleak.VerifyTestMain(m) }
```

**Go's `-race` is the best of the three race detectors.** Always use it during development.

### 2.8 Decision matrix — which primitive when

| Need | Reach for | Why |
|---|---|---|
| Bounded buffer | `make(chan T, N)` | A channel **is** a bounded buffer |
| Mutex | `sync.Mutex` (or `chan struct{}` cap 1 if you want channel semantics) | Both work; sync.Mutex is faster |
| Reader-writer | `sync.RWMutex` | Built-in, writer-preference |
| Wait-on-predicate | `sync.Mutex + sync.Cond` (manually `for !pred { cv.Wait(...) }`) | No predicate overload; explicit loop |
| Counting semaphore | `make(chan struct{}, N)` | Pattern B scales |
| Done / broadcast cancellation | `chan struct{}` + `close()` | One line |
| Single-use latch | `sync.WaitGroup` | Counts down |
| Cyclic barrier | `sync.Mutex + sync.Cond` + generation counter | NOT WaitGroup |
| Daemon-orchestrated protocol | goroutine + channels | Single-goroutine ownership ⇒ no mutex needed |
| FIFO semaphore | daemon goroutine + `chan chan struct{}` for acquireCh + bare `chan struct{}` for releaseCh | FIFO comes from Go's per-channel FIFO guarantee |

### 2.9 The Go failure-mode philosophy

| Mistake | C++ | Go |
|---|---|---|
| Over-release a semaphore | UB (silent corruption) | Send blocks forever (deterministic, findable with SIGQUIT) |
| Use a sync primitive from "the wrong thread" | UB | Fine — channels have no ownership concept |
| Missing initialization | Possibly UB / depends | Deterministic deadlock on first op |
| Send on closed channel | n/a | **Runtime panic (loud)** |

> **Go panics at runtime where C++ silently misbehaves.** This is harder to cover with testing (panics crash the program loud) but easier to debug (every failure has a stack trace pointing at the misuse).

---

## 3. Rust patterns

### 3.1 Primitive cheat sheet

| Primitive | Module | Async? | What it gives you |
|---|---|---|---|
| `std::sync::Mutex<T>` | `std::sync` | no | mutex with poisoning. RAII guard. **`MutexGuard` is `!Send`.** |
| `std::sync::RwLock<T>` | `std::sync` | no | reader-writer lock. RAII guards. |
| `std::sync::Condvar` | `std::sync` | no | parking + notification. `wait_while` predicate API. |
| `std::sync::atomic::AtomicI32` etc. | `std::sync::atomic` | no | per-op atomicity. **Every op requires explicit `Ordering`** — no defaults. |
| `Arc<T>` | `std::sync` | no | atomically reference-counted shared pointer. Spatial sharing. |
| `tokio::sync::Mutex<T>` | `tokio::sync` | yes | async-aware mutex. Use only when guard held across `.await`. |
| `tokio::sync::Semaphore` | `tokio::sync` | yes | counting semaphore with **FIFO wakeup**. Permits are RAII. |
| `tokio::sync::Barrier` | `tokio::sync` | yes | reusable phase synchronization. |
| `tokio::sync::Notify` | `tokio::sync` | yes | wakeup signal. `notify_one()` deposits a permit; next `notified().await` consumes it (memory!). |
| `tokio::sync::oneshot` | `tokio::sync` | yes | single-shot channel. One-message addressed delivery. |
| `tokio::sync::mpsc` | `tokio::sync` | yes | async multi-producer single-consumer. |
| `tokio::sync::watch` | `tokio::sync` | yes | latest-value broadcast. |
| `std::sync::mpsc` | `std::sync` | no | sync mpsc. **`Receiver` is NOT `Clone`** — wrap in `Arc<Mutex<Receiver>>` for multi-consumer. |

### 3.2 The two-axis sharing model

> **`Arc<T>` = spatial sharing across threads. `Mutex<T>` = temporal exclusion. `Arc<Mutex<T>>` does both.**

`Arc<T>` only hands out `&T`. To MUTATE through an `Arc`, the wrapped type needs **interior mutability**:

| Inner type | Call that mutates | Gives you |
|---|---|---|
| `Mutex<T>` | `.lock()` | `MutexGuard<T>` derefs to `&mut T` |
| `RwLock<T>` | `.read()` / `.write()` | `&T` / `&mut T` |
| `AtomicI32` | `.fetch_add(...)` | atomic op through `&self` |
| `RefCell<T>` | `.borrow_mut()` | `RefMut<T>` — single-threaded only |

### 3.3 Steady-state idioms

#### A. Lock-and-mutate

```rust
let mut guard = self.state.lock().unwrap();      // *** named binding ***
guard.count += 1;                                 // mutate THROUGH guard
// guard drops at end of scope — lock released
```

Operators don't auto-deref through smart pointers; methods do:
```rust
guard.method()                                    // works (auto-deref)
guard.field += 1                                  // works (DerefMut + field access)
*guard == X                                       // *MUST DEREF — operator on outer type fails*
*guard < CAPACITY                                 // same
```

`*` belongs at the **use site**, not the binding. Never `let mut *g = mutex.lock()`.

#### B. Per-thread Arc clone

```rust
let counter = Arc::new(AtomicI32::new(0));
for _ in 0..N {
    let counter = Arc::clone(&counter);          // clone PER spawn; shadow outer name
    thread::spawn(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    });
}
```

`thread::spawn(move || ...)` moves captured variables in. Cloning per-iteration before the closure ensures each thread gets its own handle.

#### C. Condvar wait_while (predicate overload)

```rust
let mut state = self.state.lock().unwrap();
let _guard = self.cv
    .wait_while(state, |s| gen == s.generation)   // wait WHILE pred true
    .unwrap();                                      // *** named binding (let_underscore_lock) ***
```

**Polarity is OPPOSITE C++:** Rust `wait_while` waits *while* `pred` is true; C++ `wait(lock, pred)` waits *until* `pred` is true. Negate when porting.

The verbose form (manual loop) — also valid:
```rust
while !pred(&state) {
    state = cv.wait(state).unwrap();
}
```

#### D. Tokio Semaphore RAII

```rust
let permit = sem.acquire().await.unwrap();
// hold while in scope — release on Drop
// no .release() method exists
```

For C-style P/V (signal that must "stick"):
```rust
sem.acquire().await.unwrap().forget();           // P — consume permit, no Drop refund
sem.add_permits(1);                               // V — manually issue a permit
```

#### E. LIFO scope-exit drop = reverse-acquisition release

```rust
let _turnstile = self.turnstile.acquire().await.unwrap();    // declared 1st
let _room      = self.room_empty.acquire().await.unwrap();   // declared 2nd
// ... write ...
}   // _room drops first (LIFO), then _turnstile — free reverse-order release
```

**Order of `let` bindings encodes release order — for free.**

#### F. `.await` is postfix syntax

```rust
expr.await       // ← correct, no parens
expr.await()     // ← compile error: "await is not a method call"
```

`.await` and `?` are the two postfix-keyword expressions in Rust.

### 3.4 Async / `Send`-bound traps

#### A. Holding a `MutexGuard` across `.await` — `!Send` future

```rust
let guard = self.map.lock().unwrap();             // !Send
do_async_thing().await;                            // ← future is now !Send → tokio::spawn rejects
```

**Block-scope** the guard so it's lexically dead before the next `.await`:
```rust
let result = {
    let guard = self.map.lock().unwrap();
    guard.compute()
};   // *** guard dies HERE, lexically, before the next .await ***
do_async_thing(result).await;
```

**`drop(guard)` does NOT help** — the analysis is **lexical**, not dataflow. The compiler conservatively assumes any name in scope is alive at the next await.

#### B. `std::sync::Mutex` vs `tokio::sync::Mutex`

| Use | When |
|---|---|
| `std::sync::Mutex` | Critical section is short and contains NO `.await`. Faster (no async overhead). |
| `tokio::sync::Mutex` | Need to hold the lock across `.await`. Integrates with the runtime. |

The tokio docs explicitly recommend `std::sync::Mutex` for short sync-only critical sections.

#### C. `permit.forget()` + `add_permits()` for cross-task lifetimes

```rust
// First reader (one task):
let permit = self.room_empty.acquire().await.unwrap();
permit.forget();                                  // skip Drop — semaphore stays decremented

// Last reader (a DIFFERENT task, much later):
self.room_empty.add_permits(1);                   // manually put a permit back
```

When acquire and release are in **different tasks/futures**, RAII can't bridge — `forget()` + `add_permits()` does.

### 3.5 Pool vs gate dichotomy (the crucial Rust+tokio insight)

Same `tokio::sync::Semaphore`, opposite semantics:

| Idiom | Init | `add_permits` | `forget()`? |
|---|---|---|---|
| **Resource pool** (footman, conn pool, rate limit) | `new(capacity)` | never | **no** — let Drop refund |
| **Gate / one-shot signal** (barrier, broadcast release) | `new(0)` | every round | **yes** — consume the ticket |

- Pool: count means "how many resources free." Must oscillate. RAII keeps it honest.
- Gate: count means "how many tickets for next round." Tickets consumed, never refunded.

**Pick wrong → livelock (gate forgets to close) or leak capacity (pool forgets to refill).**

### 3.6 Pitfalls catalogue (every Rust gotcha across all problems)

#### Syntax / grammar
1. **`.await()` with parens.** Postfix syntax, not method. `expr.await`.
2. **`acquire().await.forget()` no `.unwrap()`.** `Semaphore::acquire` returns `Result` (closeable). Always `.unwrap()` (or proper error handling).
3. **`if cond { Err(item) }` then fall-through.** `if` with no `else` evaluates to `()`. Either `return Err(item);` or use `else`.
4. **`Ok()` vs `Ok(())`.** Unit `()` is still a value; `Result<(), T>` `Ok` arm is `Ok(())`.
5. **`std::move(item)`.** Rust moves implicitly — by-value parameters already transfer ownership.
6. **Missing `;` between statements.** Last expression in block has NO `;` (returns its value); statements before it need `;`.
7. **`.empty()` on `VecDeque`.** Naming convention: `is_empty`, `is_some`, `is_none`.
8. **`for i in 0...N`.** Removed legacy syntax. Use `0..N` (exclusive) or `0..=N` (inclusive).
9. **C-style parens around `if` condition.** Idiomatic Rust: `if cond { ... }`.
10. **Stray `;` inside struct literal.** Literals take `field: value` separated by `,`. No statements.
11. **`let` statements inside struct literal.** Statements come BEFORE the literal.
12. **Field naming with trailing `_` (C++ convention).** Rust uses `pub` for visibility, not name mangling.
13. **`if (self.stop_.load())` — three C++isms.** Drop parens; rename to `stop`; pass `Ordering::SeqCst`.

#### Mutex / lock guards
14. **`lock()` without `let mut` binding.** `let mut state = m.lock().unwrap();` — never an unbound expression. Without binding, guard drops at end of statement → critical section runs UNLOCKED. **Most consequential Rust concurrency mistake.**
15. **`let _ = m.lock();`** Triggers `let_underscore_lock` lint — drops guard immediately. Use `let _guard = ...;` (named binding, leading `_`).
16. **`m.lock();`** (statement, no binding). Same problem; guard dies at `;`.
17. **`drop(guard)` to release before `.await`.** Doesn't satisfy Send analysis (lexical, not dataflow). Use block-scope.
18. **Holding `std::sync::Mutex` across `.await`.** `MutexGuard` is `!Send` → future is `!Send` → `tokio::spawn` rejects.
19. **Holding `std::sync::Mutex` reentrantly.** Not reentrant — second `lock()` deadlocks. `wait_while` consumes the guard you already hold.
20. **`*` on the binding instead of the use.** `let mut *g = ...` is a syntax error. `let mut g = ...; *g += 1;`.

#### Smart pointers / auto-deref
21. **Operators don't auto-deref through guards/Box/Arc.** `guard == X`, `guard += 1` need `*guard`. Methods do auto-deref.
22. **`Mutex<[u8; N]>` indexed as `m[pid].lock()`.** One mutex over an array, not an array of mutexes. `let g = m.lock().await; g[pid] = ...;`.
23. **Cloning `Arc` without binding (silent drop).** `Arc::clone(&x)` → bind to a name → use that name in the closure.

#### Predicate / wait API
24. **`wait_while(g, |s| gen != s.generation)`.** Polarity inversion (C++ ports). Should be `gen == s.generation` ("wait *while* not moved").
25. **Closure parameter `gen` shadows captured `gen`.** Pick `s` for guard params.
26. **Implicit move of guard into `wait_while` — use-after-move trap.** The function CONSUMES the guard (it has to, to release the lock during the wait). The return is a *fresh* guard wrapped in `LockResult`. **Shadow the binding** with the new guard; don't bind it to `_guard` and try to keep using the old name — it's been moved.
    ```rust
    // WRONG — `shared` moved on line 2; line 3 fails to compile.
    let mut shared = self.state.lock().unwrap();
    let _guard = self.cv.wait_while(shared, |s| s.draining).unwrap();
    shared.seated += 1;                                         // use of moved value

    // RIGHT — shadow `shared` with the result.
    let shared = self.state.lock().unwrap();
    let mut shared = self.cv.wait_while(shared, |s| s.draining).unwrap();
    shared.seated += 1;                                         // ✓
    ```
27. **`wait_while(g, pred)` without `.unwrap()`.** Returns `LockResult<MutexGuard>`. Forgetting `.unwrap()` binds a `LockResult` and the next field access fails.
28. **`shared.draining = True` (capitalized boolean).** Rust booleans are `true`/`false`, lowercase. `True` is undefined.
29. **`notify_one()` when many parked waiters could pass the predicate.** Silent lost wakeup for the rest. Decision rule: count how many parked waiters could pass after your state update. If >1 → `notify_all`; if exactly 1 → `notify_one`. When unsure, `notify_all` is always correct (just suboptimal). Examples: sushi-bar drain end (up to 5 can pass) → `notify_all`; producer-consumer push of one item (1 consumer can pass) → `notify_one`. See `IMPLEMENTATION_PATTERNS.md` R3 for the full cross-language table.

#### Atomics
30. **`fetch_add(...)` then read `rc` again separately.** Compound-op race. Use **return value** of `fetch_add`: `let new_rc = rc.fetch_add(1, Ord) + 1;`.
31. **`if self.rc == 1`.** Can't compare `AtomicI32` directly to `i32`. (And shouldn't even try — use the return-value pattern above.)
32. **`AtomicBool::load()` (no Ordering).** Rust requires explicit `Ordering`; default `SeqCst`.

#### Tokio Semaphore
33. **`.release()` on tokio Semaphore.** Doesn't exist. Permits return on Drop.
34. **Forgot `.forget()` on a "consumed" permit.** Permit RAII-returns at scope end. For barriers/gates, `.forget()` is mandatory.
35. **Forgot `add_permits(1)` after `.forget()`.** Permit is gone permanently; semaphore can't recover. The reciprocal is mandatory.

#### `tokio::sync::Mutex` / async
36. **`tokio::sync::Mutex::lock()` `.unwrap()` instead of `.await`.** Returns Future, not Result.
37. **`std::sync::Mutex` for a CS that crosses `.await`.** Use `tokio::sync::Mutex`.
38. **`std::thread::sleep()` inside `async fn`.** Blocks the WORKER THREAD. Use `tokio::time::sleep(d).await`.
39. **`std::sync::Barrier::wait()` inside `async fn`.** Same trap. Use `tokio::sync::Barrier`.

#### Channels
40. **`Receiver` is NOT `Clone`.** Multi-consumer needs `Arc<Mutex<Receiver<T>>>`.
41. **Forgetting `drop(tx)` in main.** `mpsc` channels close when ALL senders drop. Main holds the original `tx`. Without explicit drop, consumers hang.
42. **Holding the `Mutex<Receiver>` lock across the blocking `recv()`.** Serialises waiting on top of dequeue. **Tight-scope** with `let item = { let rx = lock.lock().unwrap(); rx.recv() };`.
43. **`tx.send(value).unwrap()` panics on cancelled receiver.** For release-style semantics, `let _ = tx.send(value);`.

#### `match` and ownership
44. **`match item` then trying to use `item` inside the arm.** `match` consumes `item`. Inside `Ok(s)`, `item` is gone; only `s` remains.

### 3.7 Build / run / debug

```bash
# Regular build
cargo build --workspace
cargo run -p <problem> --release

# ThreadSanitizer (nightly required)
RUSTFLAGS="-Z sanitizer=thread" cargo +nightly run -p <problem> --release

# On recent nightlies, sanitized user code can't link against unsanitized prebuilt std:
RUSTFLAGS="-Z sanitizer=thread" cargo +nightly run -p <problem> --release \
    -Z build-std --target x86_64-unknown-linux-gnu

# Loom — exhaustive interleaving exploration (gold standard for correctness)
# Cargo.toml: [target.'cfg(loom)'.dependencies] loom = "0.7"
RUSTFLAGS="--cfg loom" cargo test --test my_test --release

# Miri — detect UB (mostly in unsafe code, but catches some race conditions)
cargo +nightly miri run

# Stress loop
for i in {1..1000}; do cargo run -p <problem> --release --quiet || break; done
```

**Rust's compile-time push:** the borrow checker, `Send`/`Sync`, `MutexGuard !Send`, `let_underscore_lock` lint, `unused_must_use` on `Result`/`MutexGuard` — all designed to catch concurrency bugs at compile time.

### 3.8 Decision matrix — which primitive when

| Need | Reach for | Why |
|---|---|---|
| Mutex (sync) | `std::sync::Mutex` | RAII guard, faster than tokio |
| Mutex (held across `.await`) | `tokio::sync::Mutex` | Async-aware |
| Reader-writer | `std::sync::RwLock` (sync); `tokio::sync::RwLock` (async) | Built-in |
| Wait-on-predicate (sync) | `Mutex + Condvar + wait_while(g, pred)` | `wait_while`'s polarity is OPPOSITE C++ |
| Wait-on-predicate (async) | `tokio::sync::Mutex + Notify` (manual) | No predicate overload — `Notify` is the wakeup primitive |
| Counting semaphore | `tokio::sync::Semaphore` | FIFO wakeup |
| Phase synchronization | `tokio::sync::Barrier` | Built-in |
| Barrier without tokio | `Mutex + Condvar + generation` | Pure std |
| Many-producer, single-consumer (async) | `tokio::sync::mpsc` | Public mailbox |
| Many-producer, single-consumer (sync) | `std::sync::mpsc` (with `Arc<Mutex<Receiver>>` for multi-consumer) | Single-consumer by design |
| Addressed single-shot signal (async) | `tokio::sync::oneshot` | Exactly one message |
| Reusable broadcast wakeup (async) | `tokio::sync::Notify` | Has memory (deposit-permit) |
| Latest-value broadcast | `tokio::sync::watch` | New value overwrites |

---

## 4. Cross-language decision matrix

### 4.1 Same problem, what does each language do best?

| Problem | C++ | Go | Rust |
|---|---|---|---|
| Producer-consumer | Condvar (predicate explicit) or semaphore | **Channel** (literally `make(chan T, N)`) | Either (mpsc has subtle multi-consumer trap) |
| Readers-writers | `std::shared_mutex` baseline | `sync.RWMutex` baseline | `std::sync::RwLock` or `tokio::sync::RwLock` |
| Barrier | Cond + generation | **Cond + generation** (NOT WaitGroup) | Cond + generation (sync) or tokio Barrier (async) |
| Dining philosophers | `std::scoped_lock` (try-and-back-off) | Asymmetric ring (channels) or try-backoff | Asymmetric (clean tokio Mutex) |
| Barbershop | 4-semaphore protocol | 4-channel protocol | 4-tokio-Semaphore protocol |
| H2O | Semaphore + Barrier | **Daemon goroutine** or leader | Semaphore + Barrier (cleanest) or daemon |
| FIFO semaphore | Per-waiter binary_semaphore (lost-wakeup-immune) | **Daemon goroutine** (no mutex needed) | Per-ticket Notify or oneshot queue |

### 4.2 The complexity-shift table

| Primitive | Steady state | Shutdown |
|---|---|---|
| Condvar | Verbose (predicate + while + recheck) | Cheap (1 line: `notify_all`) |
| Semaphore | Cheap (acquire/release) | Verbose (release N permits per side + self-heal) |
| Channel (Go) | Cheapest (`<-`/`<-`/`range`) | "who closes" coordination + can panic loudly |
| Tokio Semaphore | Cheap (RAII) | Cross-task lifetimes need `forget()` + `add_permits()` |

### 4.3 Failure-mode philosophy

| Style | C++ | Go | Rust |
|---|---|---|---|
| Compile time | Strong type system, but no concurrency-specific checks beyond `[[nodiscard]]` | Some (`noCopy`, unused locals) | **Strongest** (`Send`/`Sync`, `MutexGuard !Send`, `let_underscore_lock`, lifetime checking) |
| Runtime | UB on misuse (silent) | Panic (loud) on most channel/WG misuse; deadlock detector for full-deadlock | Panic on `unwrap()` of unhandled errors; less runtime detection |
| Sanitizer ROI | High (TSan finds many race classes) | Highest (`-race` is best-in-class, always-on) | High (TSan + Loom for exhaustive) |
| Liveness checking | TSan does structural lock-order analysis | Built-in deadlock detector (full only) | Loom can prove invariants under all interleavings |

### 4.4 Where is FIFO wakeup guaranteed?

| Language | Primitive | FIFO? |
|---|---|---|
| C++ | `std::counting_semaphore` | **No** ("at least one" thread) |
| C++ | `std::binary_semaphore` | No (same spec) |
| C++ | `std::condition_variable` | No (unspecified) |
| Go | `chan T` (any cap) | **Yes** (spec: served in FIFO order) |
| Rust | `tokio::sync::Semaphore` | **Yes** (docs: "This Semaphore is FIFO") |
| Rust | `std::sync::Mutex` | No |

> **Same algorithm + different primitive's wakeup-fairness guarantee = different liveness behaviour.** The readers-writers turnstile is the marquee example: ~85% pass on C++ counting_semaphore, 100% on Go channels and tokio Semaphore.

---

## 5. Universal anti-patterns

These appear in EVERY language; they're the "universal traps" you must spot.

### 5.1 Conflating algorithm state with invariant oracle

**Wrong:**
```cpp
std::atomic<int> readers{0};
// reader uses 'readers' as both the algorithm count AND the invariant witness
```

**Right:** Maintain two separate variables. The oracle (e.g., `counters_.enter_read()`) is bumped only after exclusion is secured; the algorithm count (`rc_`) is bumped when accounting for queue position.

### 5.2 Per-op atomicity ≠ multi-op atomicity

**Wrong:**
```cpp
--rc_;                              // atomic
if (rc_ == 0) sem.release();        // separate op — RACES
```

**Right:** Mutex around the whole sequence, OR use `compare_exchange_weak`, OR (Rust) use `fetch_add`'s **return value**: `let new_rc = rc.fetch_sub(1, Ord) - 1;`.

### 5.3 Asymmetric bail (push vs pop)

**Wrong (loses items on shutdown):**
```cpp
not_empty_.wait(lk, [this] { return !queue_.empty() || closed_; });
if (closed_) return std::nullopt;       // discards remaining items
```

**Right:**
```cpp
if (closed_ && queue_.empty()) return std::nullopt;
```

### 5.4 Predicate polarity confusion across languages

| Library | API | Wait while...? |
|---|---|---|
| C++ | `cv.wait(lock, pred)` | `pred` is **false** (waits *until* true) |
| Rust | `cv.wait_while(guard, pred)` | `pred` is **true** (waits *while* true) |
| Go | manual `for !pred { cv.Wait() }` | explicit |

**Read the function name before the body.** Translating between C++ and Rust requires negation.

### 5.5 Lost wakeup with cv

`cv.notify_all` has **no memory**. If you mutate predicate state outside the same lock the waiter waits with, the notify can race past the unlock-and-park window → permanent hang.

**Always mutate under the lock**, even if the value is atomic.

### 5.6 Fairness assumption in liveness proofs

> Concurrency textbook proofs silently assume fair scheduling. Real primitives vary.

Before claiming starve-free, check what your primitive promises about wakeup ordering. Algorithm correctness transfers across languages; algorithm **liveness** doesn't.

### 5.7 Test BOTH paths of a dual-mode protocol

A barbershop with `Balked=0` every run isn't a passing test — it's a coverage hole. Crank contention until you see balks.

A turnstile that passes once isn't a passing test. Stress-loop with random sleeps.

### 5.8 Sanitizer cleanliness ≠ correct algorithm

TSan finds **structural** problems (data race, lock-order-inversion). It cannot reason about **runtime invariants** like "footman cap of N-1 prevents the cycle from closing." The footman strategy demonstrates this — TSan flags it, but the program is correct.

Conversely, an algorithm can pass TSan and still be wrong: the no-starve turnstile algorithm passes TSan in C++ but fails liveness ~15% of the time because of the FIFO-wakeup gap.

### 5.9 Bar for "done"

> 1000+ stress runs + sanitizer clean + invariant assertions in code + sabotage experiments confirming each invariant.
>
> NOT "passed once."

### 5.10 Sabotage experiments are the cheapest learning

Pattern: edit → predict → run → observe → revert.

For each invariant in your solution, deliberately break it and confirm what fails. If your prediction is wrong, you didn't understand the invariant. Faster than reading; sticks deeper.
