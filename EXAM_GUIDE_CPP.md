# CS3211 — C++ Exam Book

> **How to use this book during the exam.** Stay in C++ headspace. §1 is a one-page cheat sheet (which primitive lives in which header, and what each gives you). §2 is the centerpiece — full **program scaffolding** shapes you can write from blank, plus the sync idioms that go inside them. §3 collects classical problems already written in C++. §4 walks the 2025-style fill-in-the-points scenarios. §5–§7 are pitfalls, build commands, and a decision matrix.
>
> If a question gives you a half-skeleton, jump straight to §4. If it says "implement X from scratch," start at §2.

---

## Table of contents

0. [Keyword index — scenario phrase → section](#0-keyword-index)
1. [Primitive cheat sheet](#1-primitive-cheat-sheet)
2. [Idioms & scaffolding](#2-idioms--scaffolding) ← **centerpiece**
   - 2a. [Program scaffolding](#2a-program-scaffolding) (S1–S6)
   - 2b. [Sync idioms](#2b-sync-idioms) (I1–I10)
3. [Classical problems in C++](#3-classical-problems-in-c)
4. [Exam-style scenarios](#4-exam-style-scenarios)
5. [Pitfalls catalogue](#5-pitfalls-catalogue)
6. [Build / run / debug](#6-build--run--debug)
7. [Decision matrix](#7-decision-matrix)

---

## 0. Keyword index

| If the question mentions… | Look at |
|---|---|
| "thread-safe class with state" / shared registry / per-slot state | §2a S2, §4.1 (TicketSystem) |
| "lock-free" / atomic / no mutex / CAS | §2b I4, §4.2 (atomic-only TicketSystem) |
| bounded buffer / producer / consumer / queue | §3.1, §2b I2 |
| readers / writers / shared read | §3.2, §2b I8 |
| barrier / phase / round / generation | §3.3, §2b I9 |
| philosophers / chopsticks / forks / multi-resource | §3.4, §2b I3 |
| barber / chairs / balking / waiting room | §3.5 |
| H2/O / molecules / matching | §3.6 |
| FIFO / fairness / no-starve / order of arrival | §3.7 |
| timeout / lease / hold-and-expire / sweeper | §2a S4, §4.1 (monitor thread) |
| graceful shutdown / drain / stop flag | §2a S4, §2b I6, §2b I7 |
| io_uring / submission queue / completion queue / ring buffer | §4.3 (ConcurrentRing) |
| W workers / SQ-CQ / pipeline / accept loop / server | §4.4 (W-worker server) |
| concurrent tree / hand-over-hand locking / lock coupling | §4.5 (tree updates) |
| STM / transaction / atomic block / read-set / write-set | §4.6 (STM API), §4.7 (TL2 STM from A1) |

---

## 1. Primitive cheat sheet

| Primitive | Header | Default-ctor? | What it gives you |
|---|---|---|---|
| `std::mutex` | `<mutex>` | yes | mutual exclusion. **No RAII by itself.** |
| `std::lock_guard<Mutex>` | `<mutex>` | n/a | RAII for ONE mutex. Can't unlock/relock. |
| `std::unique_lock<Mutex>` | `<mutex>` | n/a | RAII for one mutex; supports unlock/relock (needed for `cv.wait`). |
| `std::scoped_lock<Mutexes...>` | `<mutex>` | n/a | RAII for one OR multiple mutexes (variadic uses `std::lock` try-and-back-off — deadlock-safe). |
| `std::shared_mutex` | `<shared_mutex>` | yes | reader-writer lock. `lock_shared()` for readers, `lock()` for writer. Pair with `std::shared_lock`. |
| `std::condition_variable` | `<condition_variable>` | yes | parking + notification. **Always pair with mutex + predicate.** |
| `std::counting_semaphore<MAX>` | `<semaphore>` | **NO** — needs init value | counter with `acquire`/`release`. **No FIFO guarantee.** **`release(n)` requires `MAX >= n`.** |
| `std::binary_semaphore` | `<semaphore>` | **NO** | alias for `counting_semaphore<1>`. Has memory — useful for signal-once. |
| `std::barrier<>` | `<barrier>` | **NO** — needs count | reusable phase synchronization. |
| `std::atomic<T>` | `<atomic>` | yes | per-op atomicity. **Default ordering: `memory_order_seq_cst`.** |
| `std::thread` | `<thread>` | yes (empty, non-joinable) | one OS thread. **Must `join()` or `detach()` before destruction or `std::terminate`.** |
| `std::this_thread::sleep_for` | `<thread>` + `<chrono>` | n/a | typed sleep. `1ms`, `500us`, `2s` literals after `using namespace std::chrono_literals;`. |

---

## 2. Idioms & scaffolding

This is the heart of the book. **§2a teaches you to write a working concurrent C++ program from a blank file.** §2b shows the sync idioms that go inside §2a's shapes. Every classical problem (§3) and exam scenario (§4) is just §2a + §2b composed.

### 2a. Program scaffolding

#### S1. The standard "main + threads + shared class" shape

The skeleton every C++ concurrency answer slots into. Memorize the headers and the `for + emplace_back + lambda + join` loop.

```cpp
#include <atomic>
#include <chrono>
#include <condition_variable>
#include <iostream>
#include <mutex>
#include <thread>
#include <vector>
// Add as needed: <deque> <optional> <semaphore> <shared_mutex>
//                <barrier> <random> <string>

using namespace std::chrono_literals;          // enables 1ms, 500us, 2s

constexpr int N_THREADS = 4;
constexpr int ROUNDS    = 100;

class Shared {
public:
    Shared() = default;                        // can also init members here
    // ... methods that callers will hit concurrently
private:
    std::mutex mu_;                            // declare once, share across threads
    // other state guarded by mu_
};

int main() {
    Shared shared;                             // ONE instance, shared by ref
    std::vector<std::thread> ts;
    ts.reserve(N_THREADS);                     // optional, avoids realloc

    for (int i = 0; i < N_THREADS; ++i) {
        ts.emplace_back([&shared, id = i] {    // capture by ref + by value
            for (int r = 0; r < ROUNDS; ++r) {
                // call shared.something(id, r);
            }
        });
    }
    for (auto& t : ts) t.join();               // *** must join before main returns ***
    return 0;
}
```

**Critical invariants:**
- **One `Shared` instance.** Declare locally in `main`, capture by reference in lambdas (`[&shared, ...]`). Never declare the mutex inside a function called per-thread — stack-local mutexes give each thread its OWN mutex (no exclusion).
- **Capture `id = i` by value**, not `[&, &i]`. Loop variable `i` is shared mutable state; threads will all see the final value.
- **Always `.join()` (or `.detach()`) before the `std::thread` destructor runs.** A non-joined non-detached thread → `std::terminate` at scope exit.

#### S2. Class with mutex + state (the canonical "shared structure")

This is the shape **every** "implement a thread-safe X" question reduces to. Mutex + (optional) condvar + (optional) atomics, methods that take RAII guards at entry.

```cpp
class Buffer {                                 // any thread-safe class
public:
    explicit Buffer(std::size_t cap) : cap_(cap) {}

    bool push(int item) {
        std::unique_lock<std::mutex> lk{mu_};  // *** function-entry guard ***
        not_full_.wait(lk, [this]{
            return queue_.size() < cap_ || closed_;
        });
        if (closed_) return false;
        queue_.push_back(item);
        not_empty_.notify_one();
        return true;
    }                                          // lk destructs here → unlocks

    std::optional<int> pop() {
        std::unique_lock<std::mutex> lk{mu_};
        not_empty_.wait(lk, [this]{
            return !queue_.empty() || closed_;
        });
        if (queue_.empty()) return std::nullopt;   // closed AND empty → bail
        int x = queue_.front();
        queue_.pop_front();
        not_full_.notify_one();
        return x;
    }

    void close() {
        { std::scoped_lock lk{mu_}; closed_ = true; }   // brace-scope the lock
        not_full_.notify_all();                         // notify OUTSIDE the lock OK
        not_empty_.notify_all();
    }

private:
    std::size_t cap_;
    std::deque<int> queue_;
    std::mutex mu_;
    std::condition_variable not_full_, not_empty_;
    bool closed_ = false;                              // mutate ONLY under mu_
};
```

**Shape rules to memorize:**
- **All mutable state goes in `private:`. The mutex sits next to the state it guards.** One mutex per "logical resource" — don't share a mutex across unrelated state, don't split state across multiple mutexes unless you mean to.
- **Every public method that touches state opens with `std::unique_lock<std::mutex> lk{mu_};` (or `lock_guard`/`scoped_lock` if you don't need to wait).** No exceptions.
- **`closed_` (or any "shutdown" boolean) is mutated under the lock**, even if you make it `std::atomic<bool>`. The condvar's unlock-and-park atomicity is only with respect to holders of the *same* mutex.

#### S3. Multiple variants of the same class (one file, many strategies)

When a question asks "try strategy X and strategy Y," wrap each in its own free function, share a common interface (typedef'd function pointer), and invoke each from `main`:

```cpp
using EatFn = void(*)(std::size_t pid, std::mt19937& rng);

void eat_scoped_lock(std::size_t pid, std::mt19937& rng) { /* strategy 1 */ }
void eat_footman    (std::size_t pid, std::mt19937& rng) { /* strategy 2 */ }
void eat_tanenbaum  (std::size_t pid, std::mt19937& rng) { /* strategy 3 */ }

void run_strategy(const char* name, EatFn fn) {
    // reset shared state, spawn N threads each running fn, join, print stats
}

int main() {
    run_strategy("scoped_lock", eat_scoped_lock);
    run_strategy("footman",     eat_footman);
    run_strategy("tanenbaum",   eat_tanenbaum);
}
```

#### S4. Worker thread owned by the class (sweeper / monitor / barber)

Pattern for "a class that runs its own background loop." The thread's lifetime is bounded by the object: ctor spawns, dtor signals stop + joins. Use this for monitor threads (expiry sweepers, rate limiters) and for one-of-a-kind workers (the single barber).

```cpp
class TicketSystem {
public:
    explicit TicketSystem(int n) : seats_(n) {
        sweeper_ = std::thread([this]{ monitorLoop(); });
    }

    ~TicketSystem() {
        stop_.store(true);                     // 1. tell loop to exit
        if (sweeper_.joinable()) sweeper_.join();   // 2. wait for it
    }

    // copies / moves disabled — owns a thread
    TicketSystem(const TicketSystem&) = delete;
    TicketSystem& operator=(const TicketSystem&) = delete;

private:
    void monitorLoop() {
        while (!stop_.load()) {                // *** check BEFORE work AND AFTER wake ***
            std::this_thread::sleep_for(1s);
            std::scoped_lock lk{mu_};
            // sweep state, expire stale entries, etc.
        }
    }

    std::vector<Seat> seats_;
    std::mutex mu_;
    std::atomic<bool> stop_{false};
    std::thread sweeper_;                      // declare LAST — destroys FIRST
};
```

**Critical:**
- **`stop_` is `std::atomic<bool>` because it's read outside the mutex.** Reading a plain `bool` from one thread while another thread writes it = UB.
- **Disable copy/move when the class owns a thread.** Copying would either share the `std::thread` (impossible — non-copyable) or detach it (silent bug).
- **Declare the thread member LAST** so that on destruction it's destroyed FIRST… actually no, destroyed *last among declared* but you `.join()` it manually in `~Class()` before the other members go away. The `joinable()` guard handles partially-constructed objects.

#### S5. RAII guard placement — function entry, scope exit, LIFO release

```cpp
void f() {
    std::lock_guard<std::mutex> g1{mu1};       // declared 1st → released 2nd (LIFO)
    std::lock_guard<std::mutex> g2{mu2};       // declared 2nd → released 1st
    // critical section over BOTH
}                                              // g2 destructs → unlocks mu2
                                               // g1 destructs → unlocks mu1
```

For multi-mutex acquire WITHOUT manual order: `std::scoped_lock lk{mu1, mu2};` does internal try-and-back-off.

For brace-scoping the lock to a sub-region:
```cpp
void close() {
    {
        std::scoped_lock lk{mu_};
        closed_ = true;
    }                                          // lock released HERE
    cv_.notify_all();                          // notify outside the lock — fine
}
```

#### S6. Atomics-only class (no mutex)

When the question forbids mutex/cv (or asks for "lock-free"), the shape is: every field is `std::atomic`, every state transition is a CAS loop or single atomic op. No `std::mutex`, no `std::lock_guard`.

```cpp
struct Seat {
    std::atomic<bool> available;
    std::atomic<int>  heldBy;
    Seat() : available(true), heldBy(-1) {}
};

class TicketSystem {
public:
    explicit TicketSystem(int n) : seats_(n) {}

    bool reserveSeat(int userID, int seatID) {
        bool expected = true;
        if (!seats_[seatID].available.compare_exchange_strong(expected, false)) {
            return false;                      // someone else got it (expected=false now)
        }
        seats_[seatID].heldBy.store(userID, std::memory_order_release);
        return true;
    }

private:
    std::vector<Seat> seats_;
};
```

**Why `compare_exchange_strong(expected, false)`:** atomically tests `available == true` and writes `false` if so. Returns `true` iff the swap happened. `expected` is updated to the actual value on failure (so you can loop on `compare_exchange_weak` if needed — see I4).

**Why `store(..., release)` after the CAS:** ensures `heldBy` is written *after* the seat is marked taken, so any thread that later sees `available == false` (with `acquire`) is guaranteed to see the correct `heldBy`.

---

### 2b. Sync idioms

These are the snippets that fill in the shapes from §2a. Each idiom shows: when to use it, the code, and the gotcha. Each idiom is self-contained — no flipping to other files.

#### I1. Mutex + RAII guard

**When.** Default for any "exclusive access" critical section.

```cpp
std::mutex mu;
{
    std::lock_guard lk{mu};                    // C++17 CTAD — no need for <std::mutex>
    // critical section
}                                              // released here
```

**Gotchas.**
- Plain `{}` braces don't release a manually-locked mutex. Use a guard, or pair `mu.lock()` with manual `mu.unlock()` (in reverse order if you have several).
- `std::lock_guard` can't be unlocked early. If you need to release mid-scope (e.g., before a `cv.wait`), use `std::unique_lock`.

#### I2. Condvar with predicate (the canonical wait shape)

**When.** Wait until a condition becomes true. Always paired with a mutex; predicate orientation is **"once true, stop waiting."**

```cpp
std::unique_lock<std::mutex> lk{mu_};
not_empty_.wait(lk, [this]{
    return !queue_.empty() || closed_;         // *** progress disjunct OR give-up disjunct ***
});
if (closed_ && queue_.empty()) return std::nullopt;   // distinguish AFTER wake
```

The predicate overload internally loops `while (!pred()) cv.wait(lk);`. The bare form is verbose:
```cpp
while (queue_.empty() && !closed_) not_empty_.wait(lk);   // equivalent
```

**Gotchas.**
- **Use `wait(lk, pred)` not `if (pred) wait(lk)`.** Spurious wakeups are real; the predicate loop is mandatory.
- **Predicate orientation: TRUE means stop waiting.** `wait(lk, [&]{ return closed_; })` waits *until* closed → on a healthy buffer, sleeps forever.
- **Always include the give-up disjunct (`|| closed_`).** Without it, a closed empty queue blocks consumers forever.
- **`[this]` capture, not `[]`.** Bare `[]` can't see `queue_`.
- **Mutate predicate state under the lock**, even if it's an atomic. Lost-wakeup window: `cv.notify_*` has no memory.
- **`notify_all` to wake everyone; `notify_one` to wake exactly one.** When ≥2 waiters could pass after your update → `notify_all`. When unsure → `notify_all` (suboptimal but correct).

#### I3. Multi-mutex via `std::scoped_lock`

**When.** Acquire 2+ mutexes atomically without deadlock (dining philosophers, two-account transfer).

```cpp
std::scoped_lock lk{mu1, mu2};                 // internal try-and-back-off
// both held
```

**Gotchas.**
- Without `scoped_lock`, hand-locking `mu1` then `mu2` in different orders across threads = deadlock.
- For ONE mutex, prefer `lock_guard` (cheaper). `scoped_lock` is for the multi-mutex case.

**See also.** §3.4 (dining philosophers — full worked example).

#### I4. Atomic CAS loop (compare-exchange)

**When.** Lock-free state transitions on a single field. Two flavors:

**Single-shot CAS (test-and-set):**
```cpp
bool expected = true;
if (seat.available.compare_exchange_strong(expected, false)) {
    // we won the seat
} else {
    // expected now contains the actual value (false)
}
```

**Read-modify-write loop (when you need the previous value to compute the new one):**
```cpp
int cur = counter.load();
while (!counter.compare_exchange_weak(cur, cur + 1)) {
    // cur was updated to actual value; loop will retry
}
// counter incremented by exactly one, returns the old `cur`
```

**Why `_weak` in the loop, `_strong` for single-shot.** `_weak` may spuriously fail on some platforms (cheaper); fine inside a loop. `_strong` doesn't spuriously fail; use when you're not looping.

**Gotchas.**
- **Compound ops on atomic are NOT atomic.** `counter--; if (counter == 0) ...` races even with atomic counter. Use `fetch_sub`'s **return value**: `if (counter.fetch_sub(1) == 1) ...`.
- **Default `memory_order_seq_cst` is fine** unless you're optimizing. Don't pass `relaxed` unless you understand acquire/release.

**See also.** §4.2 (atomic-only TicketSystem — full worked example).

#### I5. Counting semaphore P/V

**When.** Resource pool, rate limit, capacity bound — anything you can phrase as "N permits."

```cpp
std::counting_semaphore<MAX> sem{INITIAL};

sem.acquire();           // P / wait — blocks until a permit is available
// critical section / hold permit
sem.release();           // V / signal — emit one permit
sem.release(n);          // emit n permits (for shutdown broadcast)
```

**Gotchas.**
- **No default constructor.** Always initialize: `counting_semaphore<10> s{0};`.
- **`<MAX>` is the template argument** (max permit count); `{INITIAL}` is the runtime initial count. **`release(n)` requires MAX ≥ n.**
- **No FIFO guarantee.** Spec says "at least one thread unblocks" — order is unspecified.
- For shutdown broadcast: `sem.release(N_WAITERS)` + each woken waiter checks a `closed_` flag and re-releases the permit it consumed if it bails (self-healing — see I6).

#### I6. Self-healing semaphore bail (shutdown)

**When.** You woke up on `acquire()`, but the program is shutting down. You consumed a permit you don't need; re-release so another waiter can also wake-and-bail.

```cpp
bool push(T item) {
    spaces_.acquire();
    if (closed_.load()) {
        spaces_.release();                     // *** put back the permit you took ***
        return false;
    }
    // ... real work ...
}

void close() {
    closed_.store(true);
    spaces_.release(NUM_PRODUCERS);            // wake every potentially-parked producer
    items_.release(NUM_CONSUMERS);             // wake every potentially-parked consumer
}
```

**Gotchas.**
- Forgetting to re-release on the bail path: permits leak; later operations underflow.
- Sizing the close broadcast smaller than the actual waiter count: deadlock.

#### I7. Worker loop with shutdown — recheck after wake

**When.** Long-running worker (barber, monitor thread) that parks on a semaphore or condvar.

```cpp
void barber() {
    while (true) {
        customer_sem_.acquire();
        if (stop_.load()) break;               // *** RECHECK between wake and work ***
        // serve customer
    }
}

void stop() {
    stop_.store(true);
    customer_sem_.release();                   // wake the worker so it sees stop_
}
```

**Gotcha.** A wake could be "real work arrived" or "exit now." You can't tell from the wake itself; the recheck disambiguates. Without it, the worker processes a phantom customer.

#### I8. Reader-writer baseline (`std::shared_mutex`)

**When.** Many concurrent readers, occasional writer. Built-in is heavily optimised; reach for it before hand-rolling.

```cpp
#include <shared_mutex>
std::shared_mutex mu_;
std::unordered_map<int, int> data_;

int read(int k) {
    std::shared_lock lk{mu_};                  // many readers in parallel
    return data_.at(k);
}

void write(int k, int v) {
    std::unique_lock lk{mu_};                  // exclusive
    data_[k] = v;
}
```

**Gotchas.**
- Writer preference / fairness is implementation-defined. If the question says "no writer starvation," roll your own with the lightswitch + turnstile pattern (see §3.2).
- `std::shared_lock` for read access (NOT `std::lock_guard`).

**See also.** §3.2 (readers-writers — including a no-starve hand-rolled variant).

#### I9. Cyclic barrier — generation counter

**When.** N threads must finish phase R before any starts phase R+1. `std::barrier<>` exists (C++20) but the generation-counter pattern is the production shape and what you'd write if asked.

```cpp
class Barrier {
public:
    explicit Barrier(int n) : n_(n) {}

    void wait() {
        std::unique_lock<std::mutex> lk{mu_};
        int my_gen = generation_;
        if (++arrived_ == n_) {
            arrived_ = 0;
            ++generation_;                     // *** advance generation BEFORE notify ***
            cv_.notify_all();
        } else {
            cv_.wait(lk, [&]{ return generation_ != my_gen; });
        }
    }
private:
    int n_, arrived_ = 0, generation_ = 0;
    std::mutex mu_;
    std::condition_variable cv_;
};
```

**Gotcha.** "One-lap-ahead bug": using a `bool ready_` instead of an integer generation lets a fast thread re-arrive at the barrier in round R+1 before slower threads have left round R, and steal their wakeup. The integer generation eliminates the ambiguity.

**See also.** §3.3 (barrier — full worked example).

#### I10. Atomic counter publication (the "publish on commit" pattern)

**When.** A reader needs to observe completed writes only. Producer fills a slot, then bumps a `std::atomic<size_t> size_` with `release`; reader reads `size_` with `acquire`, then is guaranteed to see all stores ordered before the bump.

```cpp
std::vector<int> data_(MAX);
std::atomic<std::size_t> size_{0};

void producer_append(int x) {
    auto i = size_.load(std::memory_order_relaxed);
    data_[i] = x;                              // plain write — safe because no reader sees it yet
    size_.store(i + 1, std::memory_order_release);   // *** publication ***
}

int consumer_read(std::size_t i) {
    auto sz = size_.load(std::memory_order_acquire);  // *** matches release ***
    if (i >= sz) return -1;
    return data_[i];                                  // safe — release/acquire orders
}
```

**Gotcha.** Single-producer only. For multi-producer, you need CAS on `size_`.

#### I11. Binary semaphore as signal-with-memory

**When.** Per-waiter wakeup that must "stick" even if the signaler fires before the waiter parks. The classic application is FIFO-fair release, where each waiter has its own private one-shot signal.

```cpp
#include <semaphore>

struct Waiter { std::binary_semaphore sem{0}; };  // init 0 — first acquire blocks

// signaler (any thread):
waiter->sem.release();

// waiter — works EVEN IF release fired earlier:
waiter->sem.acquire();
```

**Why "memory" matters.** A condition variable's `notify` evaporates if no one is parked. A binary semaphore's `release` *deposits* the permit; the next `acquire` consumes it without blocking. This is what makes per-waiter binary semaphores immune to lost wakeups.

**Gotchas.**
- `std::binary_semaphore` = `counting_semaphore<1>`. Init `0` means closed; init `1` means open.
- After one `release` and one `acquire`, the permit count is back to 0 — single-shot. For repeated signals to the same waiter, use a counting semaphore or build your own ticket queue.
- Per-waiter binary semaphores are heavier than a shared cv: O(N) objects, O(N) `release()` calls on shutdown. Worth it only when fairness/lost-wakeup-immunity matters.

**See also.** §3.7 (FIFO semaphore — strategy that builds on this).

---

## 3. Classical problems in C++

Each problem is one self-contained section: scenario → invariants (terse) → code (each variant references the §2 idioms it composes) → mistakes most worth memorizing. For algorithmic depth (English-first walkthrough, failure-mode tables) consult `EXAM_GUIDE_PER_PROBLEM.md`.

### 3.1 Producer–consumer (bounded buffer)

**Scenario.** N producers push items into a bounded buffer of capacity K; M consumers pop items. Either side blocks if the buffer is full / empty. Shut down cleanly: every produced item is consumed.

**Invariants.** `0 ≤ size ≤ capacity`; `produced == consumed` after shutdown.

#### Variant A — condvar (mutex + 2× condition_variable)

Composes **S2** + **I2** + **I6** (asymmetric bail).

```cpp
template <typename T>
class BoundedBuffer {
public:
    explicit BoundedBuffer(std::size_t cap) : capacity_(cap) {}

    bool push(T item) {
        std::unique_lock<std::mutex> lk{mut_};
        not_full_.wait(lk, [this]{
            return queue_.size() < capacity_ || closed_;
        });
        if (closed_) return false;
        queue_.push_back(std::move(item));
        not_empty_.notify_one();
        return true;
    }

    std::optional<T> pop() {
        std::unique_lock<std::mutex> lk{mut_};
        not_empty_.wait(lk, [this]{
            return !queue_.empty() || closed_;
        });
        if (closed_ && queue_.empty()) return std::nullopt;   // *** ASYMMETRIC bail ***
        T item = std::move(queue_.front());
        queue_.pop_front();
        not_full_.notify_one();
        return item;
    }

    void close() {
        { std::scoped_lock lk{mut_}; closed_ = true; }
        not_full_.notify_all();
        not_empty_.notify_all();
    }

private:
    std::size_t capacity_;
    std::deque<T> queue_;
    std::mutex mut_;
    std::condition_variable not_full_, not_empty_;
    bool closed_ = false;                                     // protected by mut_
};
```

**Mistakes worth memorizing.**
1. Inverted predicate `wait(lk, [&]{ return closed_; })` — sleeps forever on a healthy buffer.
2. `pop` bails on `closed_` alone — drops the in-flight tail. Bail rule: pop drains first; only return `nullopt` when `closed_ && queue.empty()`.
3. `notify_one` after `push` is correct (only one consumer can pass `!empty`); `notify_all` from `close` is mandatory (every parked thread must wake).

#### Variant B — semaphore (counting × 2 + mutex)

Composes **I5** (P/V) + **I6** (self-healing bail).

```cpp
constexpr int MAX_PRODUCERS_WAITERS = NUM_PRODUCERS;
constexpr int MAX_CONSUMERS_WAITERS = NUM_CONSUMERS;

template <typename T>
class BoundedBuffer {
public:
    explicit BoundedBuffer(std::size_t cap)
      : capacity_(cap),
        spaces_sem_(static_cast<std::ptrdiff_t>(cap)),
        items_sem_(0) {}

    bool push(T item) {
        spaces_sem_.acquire();
        if (closed_.load()) { spaces_sem_.release(); return false; }      // self-heal
        { std::scoped_lock lk{mut_}; queue_.push_back(std::move(item)); }
        items_sem_.release();
        return true;
    }

    std::optional<T> pop() {
        items_sem_.acquire();
        std::unique_lock<std::mutex> lk{mut_};
        if (closed_.load() && queue_.empty()) {
            items_sem_.release();                                          // self-heal
            return std::nullopt;
        }
        T item = std::move(queue_.front()); queue_.pop_front();
        spaces_sem_.release();
        return item;
    }

    void close() {
        closed_.store(true);
        spaces_sem_.release(MAX_PRODUCERS_WAITERS);
        items_sem_.release(MAX_CONSUMERS_WAITERS);
    }

private:
    std::size_t capacity_;
    std::deque<T> queue_;
    std::mutex mut_;
    std::counting_semaphore<> spaces_sem_;
    std::counting_semaphore<> items_sem_;
    std::atomic<bool> closed_{false};                          // *** atomic — read outside lock ***
};
```

**Mistakes worth memorizing.**
1. Plain `bool closed_` read outside the lock = UB. Use `std::atomic<bool>`.
2. `close()` only flipping the flag — semaphores have NO broadcast. You MUST `release(N)` per side.
3. Forgetting the self-heal `release()` on the closed-bail path: permits leak; later operations underflow.

---

### 3.2 Readers–writers

**Scenario.** Many concurrent readers, occasional exclusive writers, over a shared map / cache.

**Invariants.** `readers > 0 → writers == 0`; `writers > 0 → readers == 0 && writers == 1`.

> Maintain the witness counters in an oracle struct **separate** from the algorithm's `rc_`. Bump only AFTER you've actually secured exclusion.

#### Variant A — `std::shared_mutex` baseline

Composes **I8**.

```cpp
class KVCache {
    std::shared_mutex mu_;
    std::unordered_map<std::string, std::string> map_;
public:
    std::string get(const std::string& k) {
        std::shared_lock lk{mu_};
        auto it = map_.find(k);
        return it != map_.end() ? it->second : "";
    }
    void set(std::string k, std::string v) {
        std::unique_lock lk{mu_};
        map_[std::move(k)] = std::move(v);
    }
};
```

#### Variant B — hand-rolled Lightswitch (reader-preference, starves writers)

Composes **I5** with the lightswitch idiom: first-in-acquires / last-out-releases.

```cpp
class KVCache {
    std::counting_semaphore<1> bibo_{1};         // mutex over rc_
    std::counting_semaphore<1> roomEmpty_{1};    // held while ≥1 reader OR 1 writer in
    int rc_ = 0;                                 // PLAIN int — guarded by bibo_
    std::unordered_map<std::string,std::string> map_;
public:
    std::string get(const std::string& k) {
        bibo_.acquire();
        ++rc_;
        if (rc_ == 1) roomEmpty_.acquire();      // first reader locks out writers
        bibo_.release();
        // -- read, no locks --
        auto it = map_.find(k);
        std::string r = it != map_.end() ? it->second : "";
        bibo_.acquire();
        --rc_;
        if (rc_ == 0) roomEmpty_.release();      // last reader yields
        bibo_.release();
        return r;
    }
    void set(const std::string& k, const std::string& v) {
        roomEmpty_.acquire();
        map_[k] = v;
        roomEmpty_.release();
    }
};
```

#### Variant C — no-starve turnstile

Add a third semaphore `turnstile_{1}`. Writers `acquire` it before `roomEmpty_.acquire` and `release` at the end (after `roomEmpty_.release`). Readers do an `acquire`-then-immediate-`release` of the turnstile as their first action.

```cpp
std::string get(const std::string& k) {
    turnstile_.acquire(); turnstile_.release();   // gate-pass
    // ... lightswitch as above ...
}
void set(const std::string& k, const std::string& v) {
    turnstile_.acquire();                          // *** held across roomEmpty ***
    roomEmpty_.acquire();
    map_[k] = v;
    roomEmpty_.release();
    turnstile_.release();
}
```

**Mistakes worth memorizing.**
1. `--rc_; if (rc_ == 0) sem.release();` outside the lock — compound-op race, UB. Atomic `rc_` does NOT fix it. The mutex must wrap **the whole sequence**.
2. Conflating algorithm state (`rc_`) with witness counters (`counters_`). They differ during the "first-reader-blocked" window.
3. Turnstile passes once but flakes ~15% under stress: `std::counting_semaphore` does NOT promise FIFO wakeup. Liveness ≠ correctness; same algorithm is bulletproof in Go and Rust.

---

### 3.3 Barrier

**Scenario.** N threads must all reach a sync point before any may proceed. Reusable across rounds.

**Invariants.** No thread enters round R+1 while any thread is still in round R.

#### Variant A — mutex + condvar + generation counter (production-grade)

Composes **S2** + **I9**.

```cpp
class MyBarrier {
public:
    explicit MyBarrier(std::size_t n) : expected_(n) {}

    void arrive_and_wait() {
        std::unique_lock<std::mutex> lk{mu_};
        const std::uint64_t gen = generation_;
        if (++count_ == expected_) {
            count_ = 0;
            ++generation_;                       // *** advance before notify ***
            cv_.notify_all();
            return;
        }
        cv_.wait(lk, [this, gen]{ return gen != generation_; });
    }
private:
    std::mutex mu_;
    std::condition_variable cv_;
    std::size_t expected_, count_ = 0;
    std::uint64_t generation_ = 0;               // monotonic round tag
};
```

#### Variant B — preloaded counting semaphore

Init `counting_semaphore<N> sem{0};`. Last arriver `release(N)`. Each thread `acquire()` once. Reusable for free — N permits go in, N come out, count returns to 0.

```cpp
std::counting_semaphore<N_THREADS> sem{0};
std::atomic<int> arrived{0};
// each thread:
if (arrived.fetch_add(1) + 1 == N_THREADS) {
    arrived.store(0);
    sem.release(N_THREADS);                      // requires <MAX> ≥ N
}
sem.acquire();
```

**Critical:** `release(n)` on `counting_semaphore<1>` is **UB**. Template arg must be ≥ N.

**Mistakes worth memorizing.**
1. `bool ready_` instead of generation counter: round R thread sees `ready=true`, exits, but predicate can't tell from round R+1's `ready=true`. One-lap-ahead bug.
2. `std::barrier<> b;` without count argument — no default constructor.

---

### 3.4 Dining philosophers

**Scenario.** N philosophers, N chopsticks; each needs both neighbors' chopsticks to eat. Avoid deadlock, livelock, starvation.

**Invariant.** No two adjacent philosophers eat simultaneously.

#### Strategy A — `std::scoped_lock` (try-and-back-off)

Composes **I3**.

```cpp
std::array<std::mutex, N> chopsticks;            // *** GLOBAL — must be shared ***

void eat(std::size_t pid) {
    auto left = pid, right = (pid + 1) % N;
    std::scoped_lock lk{chopsticks[left], chopsticks[right]};   // deadlock-safe
    // EATING
}
```

#### Strategy B — footman semaphore

Composes **I5**: cap concurrent eaters at N-1 (pigeonhole — no full cycle can form).

```cpp
std::counting_semaphore<> num_eaters{N - 1};
std::array<std::mutex, N> chopsticks;

void eat(std::size_t pid) {
    auto left = pid, right = (pid + 1) % N;
    num_eaters.acquire();
    chopsticks[left].lock();
    chopsticks[right].lock();
    // EATING
    chopsticks[right].unlock();                  // reverse-of-acquire
    chopsticks[left].unlock();
    num_eaters.release();
}
```

**TSan note.** TSan flags this as lock-order-inversion because the *graph* still has a cycle — the cap is a runtime prevention, not a structural one. The program is correct; the warning is structural.

#### Strategy C — Tanenbaum (state-tracking, deadlock-free, **starvation-prone**)

Composes **S2** + **I2**, no chopstick locks at all.

```cpp
std::array<State, N> algo_states;                // guarded by m
std::mutex m;
std::array<std::condition_variable, N> cvs;

void test(std::size_t pid) {                     // PRECONDITION: holds m
    auto left = (pid + N - 1) % N, right = (pid + 1) % N;
    if (algo_states[pid] == State::HUNGRY
     && algo_states[left] != State::EATING
     && algo_states[right] != State::EATING) {
        algo_states[pid] = State::EATING;        // *** ASSIGNMENT, not == ***
        cvs[pid].notify_one();                   // notify yourself
    }
}

void eat(std::size_t pid) {
    {
        std::unique_lock lk{m};
        algo_states[pid] = State::HUNGRY;
        test(pid);
        cvs[pid].wait(lk, [pid]{ return algo_states[pid] == State::EATING; });
    }
    // EATING — mutex released, but state holds my place
    {
        std::unique_lock lk{m};
        algo_states[pid] = State::THINKING;
        test((pid + N - 1) % N);                 // give left a chance
        test((pid + 1) % N);                     // give right a chance
    }
}
```

**Mistakes worth memorizing.**
1. `std::array<std::mutex, N>` declared INSIDE `eat()` — stack-local mutexes, no exclusion. Promote to namespace/class scope.
2. `chopsticks[left].lock()` without matching `unlock()` — `std::mutex` has NO RAII semantics on its own.
3. Tanenbaum: `algo_states[pid] == State::EATING;` (typo `==` for `=`) — comparison-as-statement, total deadlock on first meal.

---

### 3.5 Barbershop (sleeping barber)

**Scenario.** One barber, K chairs in waiting room. Customer balks if all chairs full; otherwise sits, waits, gets cut, leaves. Two-handshake protocol from slide 44.

**Invariant.** `customers ≤ CHAIRS`; served + balked == TOTAL_CUSTOMERS.

#### Implementation — 4-semaphore protocol + counter mutex

Composes **S4** (worker thread) + **I5** (4 semaphores) + **I7** (recheck after wake).

```cpp
class Barbershop {
    std::counting_semaphore<> customer_sem_{0};        // customer→barber: "I'm here"
    std::counting_semaphore<> barber_sem_{0};          // barber→customer: "your turn"
    std::counting_semaphore<> customer_done_sem_{0};   // customer→barber: "I'm done"
    std::counting_semaphore<> barber_done_sem_{0};     // barber→customer: "you can leave"
    std::mutex mut_;
    std::atomic<bool> stop_{false};
    int customers_ = 0;                                // protected by mut_
public:
    bool customer(int id) {
        {
            std::scoped_lock lk{mut_};
            if (customers_ == CHAIRS) return false;    // BALK
            ++customers_;
        }
        customer_sem_.release();                       // front handshake half 1
        barber_sem_.acquire();                         // front handshake half 2
        std::this_thread::sleep_for(1ms);              // haircut
        customer_done_sem_.release();                  // back handshake half 1
        barber_done_sem_.acquire();                    // back handshake half 2
        { std::scoped_lock lk{mut_}; --customers_; }
        return true;
    }

    void barber() {
        while (true) {
            customer_sem_.acquire();
            if (stop_.load()) break;                   // *** RECHECK after wake ***
            barber_sem_.release();
            std::this_thread::sleep_for(1ms);          // cut
            customer_done_sem_.acquire();
            barber_done_sem_.release();
        }
    }

    void stop() {
        stop_.store(true);
        customer_sem_.release();                       // wake the parked barber
    }
};
```

**Mistakes worth memorizing.**
1. Direction inversion: writing `barber_sem_.acquire()` where you meant `release()` (or vice versa). Read aloud: "acquire" = wait, "release" = signal.
2. Per-semaphore, exactly one V'er and one P'er. If both sides do the same op, no synchronization happens.
3. Top-of-loop `stop_` check ALONE: after `stop()` releases `customer_sem_`, the barber's loop top has already passed for this iteration → proceeds into `barber_sem_.release` → blocks at `customer_done_sem_.acquire`. Recheck **after** the wake.
4. Default config (CHAIRS=3, fast cuts) → `Balked=0` every run. Crank contention until balks happen.

---

### 3.6 H₂O (water factory)

**Scenario.** Continuous stream of H and O atoms; each is a thread that calls `bond_h` or `bond_o`. Every molecule = exactly 2H + 1O bonding together; no atom may bond unless 2H+1O have arrived.

**Invariants.** Exactly 3 atoms in `bond()` together (2H + 1O); after H_total H atoms and O_total O atoms have bonded, all are accounted for.

#### Implementation — semaphore + barrier

Composes **I5** + `std::barrier<>`.

```cpp
class WaterFactory {
    std::counting_semaphore<> hydrogen_sem_{2};   // at most 2 H past this line
    std::counting_semaphore<> oxygen_sem_{1};     // at most 1 O past this line
    std::barrier<> barrier_{3};                   // *** must construct with count ***
public:
    void hydrogen(int id) {
        hydrogen_sem_.acquire();
        barrier_.arrive_and_wait();
        bond_h(id);
        hydrogen_sem_.release();                  // *** AFTER barrier — release before bond would let a 4th H in ***
    }
    void oxygen(int id) {
        oxygen_sem_.acquire();
        barrier_.arrive_and_wait();
        bond_o(id);
        oxygen_sem_.release();
    }
};
```

**Why both primitives.** Semaphores alone → atoms can bond solo. Barrier alone → 3 oxygens can bond into ozone. Together → semaphores enforce **type count**, barrier enforces **start signal**.

**Mistakes worth memorizing.**
1. Releasing the type-cap semaphore BEFORE the barrier — a 4th atom of that type rushes in mid-bond. Hold across barrier+bond.
2. `std::barrier<> b;` without count — no default constructor (same shape as `counting_semaphore<>{N}`).

---

### 3.7 FIFO semaphore

**Scenario.** Counting semaphore with the additional guarantee that waiters are unblocked in FIFO order of their `acquire()` calls.

**Invariant.** Every `release()` does exactly ONE thing per call — wake the head waiter OR bump count. Never both.

#### Strategy A — ticket queue + condvar (cv-based, lost-wakeup-prone if mutated outside lock)

Composes **S2** + **I2** + **I4** (atomic).

```cpp
class FifoSemaphore {
    std::mutex mut_;
    std::condition_variable cv_;
    std::atomic<std::ptrdiff_t> next_ticket_{1};
    std::atomic<std::ptrdiff_t> now_serving_;
public:
    explicit FifoSemaphore(std::ptrdiff_t initial) : now_serving_(initial) {}

    void acquire() {
        std::unique_lock lk{mut_};
        auto my_ticket = next_ticket_.fetch_add(1);
        cv_.wait(lk, [my_ticket, this]{ return my_ticket <= now_serving_; });
    }

    void release() {
        { std::scoped_lock lk{mut_}; ++now_serving_; }       // *** mutation under lock ***
        cv_.notify_all();
    }
};
```

#### Strategy B — queue of per-waiter binary semaphores (lost-wakeup-immune)

Composes **I11** (binary semaphore as signal-with-memory).

```cpp
struct Waiter { std::binary_semaphore sem{0}; };

class FifoSemaphore {
    std::mutex mut_;
    int count_;
    std::queue<std::shared_ptr<Waiter>> waiters_;
public:
    explicit FifoSemaphore(int initial) : count_(initial) {}

    void acquire() {
        auto waiter = std::make_shared<Waiter>();
        {
            std::scoped_lock lk{mut_};
            if (count_ > 0) { --count_; return; }            // fast path
            waiters_.push(waiter);
        }
        waiter->sem.acquire();                                // park OUTSIDE the lock
    }

    void release() {
        std::shared_ptr<Waiter> waiter;
        {
            std::scoped_lock lk{mut_};
            if (waiters_.empty()) { ++count_; return; }      // two-branch invariant
            waiter = waiters_.front(); waiters_.pop();
        }
        waiter->sem.release();                                // direct hand-off
    }
};
```

**Mistakes worth memorizing.**
1. Mutating `now_serving_` outside the lock in Strategy A → **lost wakeup**: notify into empty cv vaporizes; waiter parks AFTER notify, never wakes. Atomic value is NOT enough — `cv.wait`'s atomic-park guarantee is only w.r.t. holders of the same mutex.
2. Strategy B's `release()` doing both branches (increment AND wake) — emits 2 permits per call, breaks count invariant.
3. `std::queue::front()` on empty → UB. The `if (empty()) bump-count; return;` branch is mandatory.

---

### 3.8 Search-Insert-Delete (three-role generalized RW)

**Scenario.** Three roles share a list. Searchers concurrent (many at once); Inserters at-most-one (compatible with searchers); Deleters exclusive (no searchers, no inserters).

**Invariants.** `0 ≤ searcher_count`; `0 ≤ inserter_count ≤ 1`; `deleter_count > 0 ⇒ searcher_count == 0 && inserter_count == 0`.

#### Implementation — three-role lightswitch (Downey 6.1.2 shape)

```cpp
class SidList {
    std::mutex mtx_;                                          // guards searcher_count_
    int searcher_count_ = 0;
    std::counting_semaphore<1> no_searcher_{1};               // held while ≥1 searcher in
    std::counting_semaphore<1> no_inserter_{1};               // held while inserter in
    std::vector<int> data_;                                   // pre-sized; never reallocates
    std::atomic<std::size_t> size_{0};
public:
    SidList() : data_(1 << 14) {}

    void search(int x) {
        mtx_.lock();
        ++searcher_count_;
        if (searcher_count_ == 1) no_searcher_.acquire();     // first-in claims
        mtx_.unlock();
        // -- actual search via I10 publication --
        std::size_t n = size_.load(std::memory_order_acquire);
        for (std::size_t i = 0; i < n; ++i) /* compare */;
        mtx_.lock();
        --searcher_count_;
        if (searcher_count_ == 0) no_searcher_.release();     // last-out yields
        mtx_.unlock();
    }
    void insert(int x) {
        no_inserter_.acquire();
        std::size_t i = size_.load(std::memory_order_relaxed);
        data_[i] = x;
        size_.store(i + 1, std::memory_order_release);        // I10 — publish
        no_inserter_.release();
    }
    void delete_one() {
        no_searcher_.acquire();                                // GLOBAL lock order: searcher → inserter
        no_inserter_.acquire();
        // -- actual delete --
        no_inserter_.release();                                // LIFO release
        no_searcher_.release();
    }
};
```

**Why no counter for inserters.** At most one inserter is ever in — the semaphore IS the count. Adding a counter is dead state. (Counter exists only when the role allows >1 concurrent participants.)

**Why both rooms are SEMAPHORES not MUTEXES.** First-acquirer ≠ last-releaser (first searcher acquires, last searcher releases — different threads). `std::mutex::unlock` from a non-locking thread is UB.

**Mistakes worth memorizing.**
1. Initialize `no_searcher_` / `no_inserter_` to **0** instead of 1 — inverted; first searcher blocks forever. The "1" means "the room is *available*."
2. Two deleters acquire `no_searcher` and `no_inserter` in opposite orders → circular-wait deadlock. Document a global lock order.
3. `std::list<T>` for storage — TSan flags `push_back` ↔ iteration as a race. Even though the abstraction permits searcher+inserter concurrency, `std::list` doesn't honor it (no release/acquire). Use the pre-allocated `vector` + atomic-size publication shape (I10).
4. Without a turnstile, deleters starve under steady searcher load — same fix as readers-writers §3.2 variant C.

---

## 4. Exam-style scenarios

These are the 2025-paper-style questions: scenario described in prose, partial skeleton given with named fill-in points. Each section shows the fill-ins, and the **patterns reused** callout points back into §2 and §3.

### 4.1 TicketSystem with mutex + state (2025 Q25)

**Scenario.** Multiple users concurrently `reserveSeat`, `processPayment`, `confirmSeat`. Reservations are time-bounded (`holdSeconds`); a sweeper expires stale holds. No double-booking.

**This is a thread-safe registry with per-slot lease + janitor thread.** It composes **S2** (class with mutex + state) + **S4** (sweeper thread owned by class).

> **2025 official answer shape.** Per-seat mutex (one mutex per seat → many users can `processPayment` on different seats in parallel) + `atomic<bool> sold` + `int reservedBy` + `expiryTime`. The shape below mirrors the official rubric.

#### Skeleton with fill-ins (Points A–I from the exam)

```cpp
// Point A — extra data structures (none needed beyond what's in B/C/D below)
#include <chrono>
#include <mutex>
#include <thread>
#include <atomic>
#include <vector>
using std::chrono::steady_clock;
using std::chrono::seconds;

struct Seat {                                            // Point B
    std::mutex mtx;                                      // *** PER-SEAT lock ***
    std::atomic<bool> sold{false};
    int reservedBy = -1;                                 // guarded by mtx
    steady_clock::time_point expiryTime =
        steady_clock::time_point::min();                 // guarded by mtx
};

class TicketSystem {
    std::vector<Seat> seats;
    // Point C — expiration thread plumbing
    std::thread expirationThread;
    std::atomic<bool> running{true};                     // sweeper stop flag
public:
    TicketSystem(int numSeats) : seats(numSeats) {       // Point D
        expirationThread = std::thread(
            &TicketSystem::monitorExpirations, this);
    }
    ~TicketSystem() {                                    // Point E
        running.store(false);
        if (expirationThread.joinable()) expirationThread.join();
    }

    bool processPayment(int userID, int seatID) {        // Point F (rubric: empty)
        confirmPayment(userID);
        return true;
    }

    bool reserveSeat(int userID, int seatID,             // Point G
                     int holdSeconds = 5) {
        if (seatID < 0 || (size_t)seatID >= seats.size()) return false;
        Seat& seat = seats[seatID];
        std::unique_lock<std::mutex> lock(seat.mtx);
        auto now = steady_clock::now();
        if (seat.sold || (seat.expiryTime > now &&
                          seat.reservedBy != userID)) {
            return false;                                // sold OR held by someone else
        }
        seat.reservedBy = userID;
        seat.expiryTime = now + seconds(holdSeconds);
        return true;
    }

    bool confirmSeat(int seatID, int userID) {           // Point H
        if (seatID < 0 || (size_t)seatID >= seats.size()) return false;
        Seat& seat = seats[seatID];
        std::unique_lock<std::mutex> lock(seat.mtx);
        auto now = steady_clock::now();
        if (seat.reservedBy == userID && seat.expiryTime > now) {
            seat.sold = true;
            seat.expiryTime = steady_clock::time_point::min();
            return true;
        }
        refundUser(userID);
        return false;
    }

    void monitorExpirations() {                          // Point I — sweep loop
        while (running.load()) {
            std::this_thread::sleep_for(seconds(1));
            auto now = steady_clock::now();
            for (auto& seat : seats) {
                std::unique_lock<std::mutex> lock(seat.mtx);
                if (!seat.sold && seat.expiryTime < now) {
                    seat.reservedBy = -1;
                    seat.expiryTime = steady_clock::time_point::min();
                }
            }
        }
    }
};
```

**Patterns reused.** S2 (class + per-seat mutex + state); S4 (sweeper thread, dtor joins); I1 (RAII guard at every method entry).

**Why parallelism is preserved.** **One mutex per seat.** Many users hitting different seats run in parallel — only contention on the *same* seat serializes. The atomic `sold` lets the read-only "is this seat for sale?" path avoid the lock entirely if the question allows it.

**Common mistakes.**
- Single global `std::mutex` for the whole `TicketSystem` — kills the parallelism the question explicitly asks for.
- Reading `seats[s]` outside the lock to "fast-path" non-atomic fields. `reservedBy` and `expiryTime` are non-atomic; mixed-mode access = UB.
- Forgetting that between `reserveSeat` and `processPayment` the sweeper could expire the hold. `confirmSeat` MUST recheck `reservedBy == userID && expiryTime > now`.
- `~TicketSystem()` without joining the sweeper → detached thread touches destroyed `seats`.
- `running` as plain `bool` — read by sweeper, written by main = UB. Use `std::atomic<bool>`.
- Forgetting to `start` the expiration thread in the constructor or to `join` it in the destructor (rubric explicitly awards marks for both).

---

### 4.2 TicketSystem atomic-only with CAS (2025 Q26)

**Scenario.** Same `TicketSystem` interface, but the question constrains: each `Seat` uses two atomic fields (`available`, `heldBy`); no mutex. Implement the start of `reserveSeat` to mark the seat unavailable and record the holder. (Time-bounded expiry handled elsewhere.)

**This is a single-shot CAS reservation.** It composes **S6** (atomics-only class) + **I4** (CAS).

#### Skeleton with fill-in (Point J from the exam)

```cpp
struct Seat {
    std::atomic<bool> available;
    std::atomic<int>  heldBy;
    Seat() : available(true), heldBy(0) {}
};

class TicketSystem {
    std::vector<Seat> seats;
public:
    bool reserveSeat(int userID, int seatID, int holdSeconds = 5) {
        steady_clock::time_point newExpiry = steady_clock::now() + seconds(holdSeconds);

        // Point J — claim the seat atomically.
        bool expected = true;
        if (!seats[seatID].available.compare_exchange_strong(
                expected, false,
                std::memory_order_acq_rel,                 // success: order subsequent stores
                std::memory_order_acquire)) {              // failure: just observe latest
            return false;                                   // someone else got the seat
        }
        seats[seatID].heldBy.store(userID, std::memory_order_release);
        // (Expiry / lease management handled elsewhere per the prompt.)
        return true;
    }
};
```

**Why CAS, not load-then-store.** A naive `if (s.available) { s.available = false; ... }` has a race window between the load and the store: two users both observe `available == true` and both write `false`, both think they succeeded. CAS atomically tests-and-sets in one op.

**Why `compare_exchange_strong` and not `_weak`.** `_weak` may spuriously fail on some platforms — fine inside a retry loop, wrong for a single-shot test-and-set. Single attempt → use `_strong`.

**Why `heldBy.store(..., release)` after the CAS.** A reader that later sees `available == false` (with `acquire`) is then guaranteed to see the correct `heldBy`. Without the release/acquire pair, the reader could see `available == false` but a stale `heldBy`.

**Patterns reused.** S6 (atomics-only); I4 (CAS); I10 (release/acquire publication of dependent state).

**Common mistakes.**
- `if (seats[seatID].available.load()) { seats[seatID].available.store(false); ... }` — race window between load and store; two threads can both succeed.
- Using `compare_exchange_weak` for a single-shot test-and-set — spurious failures cause false-negative returns.
- Forgetting the release-store of `heldBy` after the CAS — readers can see "seat taken" with a stale or default `heldBy`.
- Writing `expected = true; ... .compare_exchange_strong(true, false)` — first arg is `T&` (must be a non-const lvalue), not a literal.

---

### 4.3 ConcurrentRing — io_uring-style submission/completion ring (2023 Q6)

**Scenario.** Implement a thread-safe bounded ring (circular buffer) that supports concurrent submission and retrieval. Submissions block when the ring is full; retrievals block when the ring is empty. This is the io_uring submission queue / completion queue pattern.

**This is the "channel" of C++ — a bounded MPMC queue.** Composes **I5** (P/V) on top of a lock-free queue.

```cpp
#include <semaphore>
#include <optional>

struct Request {
    int      client_fd;
    data_t   data;
    res_t    result;
};

class ConcurrentRing {
private:
    LockFreeQueue<Request> queue;                          // from Tutorial 4
    std::counting_semaphore<SIZE> write{SIZE};             // free slots — start full
    std::counting_semaphore<SIZE> read{0};                 // queued items — start empty
public:
    void submit_request(Request req) {
        write.acquire();                                   // wait for a free slot
        queue.push(req);
        read.release();                                    // signal one item available
    }

    Request retrieve_request() {
        read.acquire();                                    // wait for an item
        std::optional<Request> req;
        while (true) {
            req = queue.try_pop();                         // lock-free queue may need retry
            if (req) break;
        }
        write.release();                                   // free a slot
        return req.value();
    }
};
```

**Patterns reused.** I5 (counting semaphore P/V); the producer-consumer §3.1 semaphore variant generalised to N producers / N consumers.

**Why two semaphores.** `write` counts free slots (init `SIZE`); `read` counts queued items (init `0`). Each `submit` consumes a free slot and emits an item; each `retrieve` consumes an item and emits a free slot. This is exactly the semaphore producer-consumer protocol, just wrapped around a lock-free queue instead of a deque.

**Why the `try_pop` loop.** A lock-free queue can momentarily fail `try_pop` even when an item is in flight (concurrent push hasn't yet linearized). The semaphore guarantees at least one item exists; the loop tolerates the lock-free queue's brief inconsistency window.

**Common mistakes.**
- Forgetting either semaphore — submission with no `write` blows past the ring size; retrieval with no `read` busy-waits or returns `nullopt`.
- Initializing both semaphores to the same value — `write` should start at `SIZE` (all slots free), `read` at `0` (no items).
- Using a plain `std::queue<Request>` + mutex for the underlying storage — works but loses the "high concurrency" optimization the question asks for. The lock-free queue is the point.

---

### 4.4 C++ server with W workers + SQ + CQ pipeline (2023 Q7)

**Scenario.** A high-performance server. Each client connection spawns a thread that reads requests and pushes them into a `ConcurrentRing` SQ. W worker threads pop from SQ, call `process()`, and push the result into a `ConcurrentRing` CQ. Another set of threads (W threads, or per-client threads) pop from CQ and `send()` the result back to the originating client.

**This is a 3-stage pipeline (read → process → send) over two MPMC rings.** Composes §4.3 (ConcurrentRing) + §2a S1 (main + threads).

```cpp
ConcurrentRing SQ, CQ;

// W process workers + W send workers
for (int i = 0; i < W; i++) {
    std::thread([&]() {
        while (true) {
            Request req = SQ.retrieve_request();
            req.process();
            CQ.submit_request(req);
        }
    }).detach();

    std::thread([&]() {
        while (true) {
            Request req = CQ.retrieve_request();
            send(req.client_fd, req);
        }
    }).detach();
}

// Per-client read thread (spawned in the accept loop)
while (true) {
    int client_fd = accept(/* ... */);
    std::thread([&, client_fd]() {
        data_t data = read(client_fd);
        while (data) {
            Request req{client_fd, data};
            SQ.submit_request(req);
            data = read(client_fd);
        }
    }).detach();
}
```

**Concurrency analysis (rubric).**
- **Concurrent tasks:** submission/retrieval to/from SQ and CQ; processing of requests; sending back to clients.
- **Synchronization:** none additional — the `ConcurrentRing` already serializes; the rest is naturally parallel.
- **Maximum parallelism:** W process workers + W send workers + N client read threads simultaneously.

**Patterns reused.** I5 (the rings), S1 (thread spawn + lambda), I7 (worker loop with shutdown — omitted in rubric for brevity but production code wants it).

**Common mistakes.**
- Putting `process()` inside a critical section guarded by a mutex — kills the W-worker parallelism. The ring's internal sync is enough.
- Single thread for both retrieve-and-send — bottlenecks at I/O. Two stages with separate worker pools is the point.
- Forgetting that `send()` to a client is itself a blocking I/O — having W *send* workers (not just W process workers) keeps the pipeline draining.

---

### 4.5 Concurrent tree updates with hand-over-hand locking (2024 Q10)

**Scenario.** A tree where each node has an `id`. Updates can run concurrently. Coarse-grained: one global lock — kills parallelism. Fine-grained: implement a thread-safe `updateNodeID` allowing many concurrent updates.

**Two valid approaches:**

#### Approach A — atomic ID + CAS (no node mutex)

If all you ever change is the integer ID, make it atomic and use CAS:

```cpp
struct Node {
    std::atomic<int> id;
    Node* parent;
    std::vector<Node*> children;
};

bool updateNodeID(Node* node, int expected, int desired) {
    return node->id.compare_exchange_strong(expected, desired);
}
```

Trade-off: works only if updates are pure ID flips. Any structural change (rewiring children) needs locks.

#### Approach B — per-node mutex + hand-over-hand (lock coupling)

When traversal must observe a stable parent while we update a child — and the tree itself can change concurrently:

```cpp
struct Node {
    std::mutex mu;
    int id;
    Node* parent;
    std::vector<Node*> children;
};

void updateNodeID(Node* root, /* path to node */ const std::vector<int>& path, int newID) {
    Node* cur = root;
    std::unique_lock<std::mutex> guard(cur->mu);            // lock root
    for (int idx : path) {
        Node* child = cur->children[idx];
        std::unique_lock<std::mutex> childGuard(child->mu); // lock child BEFORE unlocking parent
        guard.unlock();                                      // hand over
        cur = child;
        guard = std::move(childGuard);                       // grip the child
    }
    cur->id = newID;                                         // safe — we hold cur's lock
}
```

**Why hand-over-hand.** While walking from root to leaf you must always hold *some* lock so the parent can't be removed/restructured underneath you. You release the parent's lock only AFTER acquiring the child's. The two locks overlap briefly — that's the "hand-over."

**Trade-offs vs coarse-grained.**
- Coarse: one mutex; updates serialize globally.
- Hand-over-hand: many mutexes; updates on disjoint root-to-leaf paths run in parallel; updates that share a path serialize only at the shared prefix.

**Atomic-ID variant has the same disadvantage as coarse-grained?** No — atomic-ID never blocks. But it can suffer from CAS retries under contention. Coarse-grained always blocks. Hand-over-hand blocks only on shared path nodes. Each has different worst cases.

**Common mistakes.**
- Releasing the parent lock BEFORE acquiring the child — race window where the parent could rewire its children.
- Using `std::lock_guard` (can't unlock) instead of `std::unique_lock` — you need the explicit `guard.unlock()` mid-traversal.
- Forgetting that two threads walking opposite paths need consistent global lock order to avoid deadlock — usually root-first, leaf-last is the canonical order.

---

### 4.6 STM (Software Transactional Memory) — C++ API (2024 Q11)

**Scenario.** Implement four STM API calls — `stmTxnBegin()`, `stmRd(addr)`, `stmWr(addr, val)`, `stmCommit()` — that wrap an "atomic block" over `int` memory addresses. Properties required: thread-safe (no data races), atomic (commit-or-rollback), isolation (no partial writes visible), serializability, progress.

**This is optimistic concurrency control.** Transactions execute speculatively in private buffers; conflicts are detected at commit; on conflict, abort and retry.

#### Per-thread Transaction descriptor + commit-time conflict detection

```cpp
struct Transaction {
    uint64_t txn_id;
    std::set<uint32_t> read_set;                           // addrs read
    std::set<uint32_t> write_set;                          // addrs written
    std::map<uint32_t, uint32_t> write_buffer;             // staged writes (not yet visible)
};

thread_local Transaction* current_txn = nullptr;
std::atomic<uint64_t> next_txn_id{1};

// Global serializing structures (committed-set tracking)
std::mutex commit_mtx;
std::vector<Transaction*> committed_txns;                  // history for conflict detection
std::map<uint32_t, uint32_t> shared_memory;                // protected by commit_mtx

void stmTxnBegin() {
    current_txn = new Transaction{next_txn_id.fetch_add(1), {}, {}, {}};
}

uint32_t stmRd(uint32_t addr) {
    Transaction* t = current_txn;
    t->read_set.insert(addr);
    if (auto it = t->write_buffer.find(addr); it != t->write_buffer.end()) {
        return it->second;                                 // read-your-own-writes
    }
    std::scoped_lock lk{commit_mtx};
    return shared_memory.count(addr) ? shared_memory[addr] : 0;
}

void stmWr(uint32_t addr, uint32_t value) {
    Transaction* t = current_txn;
    t->write_set.insert(addr);
    t->write_buffer[addr] = value;                         // staged, not yet visible
}

bool stmCommit() {
    Transaction* t = current_txn;
    std::scoped_lock lk{commit_mtx};
    // Validate: any concurrently committed txn that wrote to my read OR write set = conflict
    for (auto* prior : committed_txns) {
        if (prior->txn_id <= t->txn_id) continue;          // earlier txns are fine
        std::set<uint32_t> overlap_r, overlap_w;
        std::set_intersection(prior->write_set.begin(), prior->write_set.end(),
                              t->read_set.begin(), t->read_set.end(),
                              std::inserter(overlap_r, overlap_r.end()));
        std::set_intersection(prior->write_set.begin(), prior->write_set.end(),
                              t->write_set.begin(), t->write_set.end(),
                              std::inserter(overlap_w, overlap_w.end()));
        if (!overlap_r.empty() || !overlap_w.empty()) {
            // CONFLICT — caller must restart the txn
            delete current_txn; current_txn = nullptr;
            return false;
        }
    }
    // Apply writes atomically
    for (auto& [addr, val] : t->write_buffer) shared_memory[addr] = val;
    committed_txns.push_back(t);
    current_txn = nullptr;
    return true;
}
```

**Patterns reused.** I1 (mutex around the global commit mechanism); I4 (atomic txn_id allocation); the "two sets — read set / write set" idiom is the OCC core.

**Conflict detection.**
- **Write-write:** another committed txn wrote to one of my write addresses → my write may be stale.
- **Write-read:** another committed txn wrote to one of my read addresses → my view is stale.
- **Read-read:** never a conflict.

**Properties this gives you.**
- *Atomicity:* writes only flush at commit; abort discards the write_buffer.
- *Isolation:* readers see `shared_memory` only when no in-flight txn has staged writes there (a stronger version uses TL2-style versioned locks — see §4.7).
- *Serializability:* the commit lock orders all commits.
- *Progress:* on abort, retry — the commit lock guarantees forward progress globally.

**Generalizing to structs (rubric bonus).** The challenges are (a) `set<addr>` becomes `set<addr-range>`; (b) `read-your-own-writes` needs structural copy, not value copy; (c) the write_buffer must be deep-copyable. One workable approach: serialize structs to byte arrays and run STM at the byte level; another: maintain shadow copies under the txn descriptor.

**Common mistakes.**
- Validating BEFORE acquiring the commit lock — race window where another txn commits between your validation and your write.
- Forgetting read-your-own-writes — `stmRd` on an address you've already `stmWr`'d should return your staged value, not shared memory.
- Conflating read_set and write_set — a txn that only writes (no reads) still needs write-write conflict detection, not skipped.

---

### 4.7 From Assignment 1 — TL2-style STM (real implementation reference)

**Scenario.** Real CS3211 assignment: implement an STM engine in C++ that handles concurrent transactions over a fixed-size memory array. The reference uses **TL2 (Transactional Locking II)**: a versioned lock per address (lock bit + version packed into one atomic), a global logical clock, and a two-phase commit.

#### Versioned-lock representation

Pack lock state and version into one `std::atomic<uint64_t>` — atomic CAS on the whole word lets you check-and-claim in one op:

```cpp
// Format: [Version (63 bits) | Lock Bit (1 bit)]
static constexpr uint64_t LOCK_BIT_MASK = 1;

bool is_locked(uint64_t lw)         { return (lw & LOCK_BIT_MASK) == 1; }
uint64_t get_version(uint64_t lw)   { return lw >> 1; }
uint64_t make_lock_word(uint64_t v) { return v << 1; }
```

#### Engine state

```cpp
class StmInterface {
    std::array<uint32_t, MEMORY_CAPACITY> _memory;
    std::array<std::atomic<uint64_t>, MEMORY_CAPACITY> _versionedLocks;
    std::atomic<uint64_t> _globalClock{0};
    // ...
};
```

#### Transaction loop (per connection)

The `ConnectionThread` reads commands (`START` / `READ` / `WRITE` / `COMMIT`); on `COMMIT` it runs a retry loop:

1. **Speculative execution.** Snapshot `rv = _globalClock.load()`. Walk the transaction log. WRITEs go to `local_writes`; READs check `local_writes` first (read-your-own-writes); otherwise they read memory between two version-load checks.
2. **Read validation during speculation.** For each non-local READ: load `v1`, read memory, load `v2`. Abort and retry if `is_locked(v1) || v1 != v2 || get_version(v1) > rv`.
3. **Lock the write-set.** For each address in `local_writes`, CAS its versioned lock from "unlocked" to "locked" (preserving the version). On CAS fail (someone else locked it), release any locks already held and retry.
4. **Determine commit timestamp.** `wv = _globalClock.fetch_add(1) + 1`.
5. **Re-validate the read-set under the locks.** Compare current version against the version recorded during speculation. If the version changed (and not by us), abort, release locks, retry.
6. **Apply writes; release locks; install new version `wv`.**

```cpp
// Step 3 — lock the write-set:
std::vector<uint32_t> locked_addresses;
for (uint32_t addr : write_addresses) {
    uint64_t cur = _versionedLocks[addr].load();
    if (is_locked(cur)) { lock_success = false; break; }
    if (!_versionedLocks[addr].compare_exchange_strong(cur, cur | LOCK_BIT_MASK)) {
        lock_success = false; break;
    }
    locked_addresses.push_back(addr);
}

// Step 4 — commit timestamp:
uint64_t commit_timestamp = _globalClock.fetch_add(1) + 1;

// Step 5 — re-validate read-set:
for (auto const& [addr, old_lw] : read_set_versions) {
    uint64_t cur = _versionedLocks[addr].load();
    bool locked_by_others = is_locked(cur) && !local_writes.count(addr);
    if (locked_by_others || get_version(cur) != get_version(old_lw)) { valid = false; break; }
}

// Step 6 — apply, install new version, release:
for (auto const& [addr, val] : local_writes) _memory[addr] = val;
for (uint32_t addr : locked_addresses)
    _versionedLocks[addr].store(make_lock_word(commit_timestamp));
```

**Why this works.** The `rv` stamp at step 1 + the version-load-double-check at step 2 guarantee that any READ either saw a consistent snapshot or aborts. The commit-time re-validation (step 5) closes the window between speculation and commit. The global clock orders all commits.

**Patterns reused (book-internal).** I4 (CAS on versioned lock); I10 (release-store on `_versionedLocks` after the writes serves as the publication boundary); §4.6 (read-set / write-set conflict detection — TL2 is the version-based realization).

**Common mistakes.**
- Validating reads ONCE during speculation but not at commit — a writer can sneak in between speculation and commit.
- Releasing the write-set locks via `store(make_lock_word(rv))` instead of `make_lock_word(commit_timestamp)` — the version doesn't advance and subsequent transactions miss the conflict.
- Acquiring write-set locks in non-sorted order across threads → deadlock by circular wait. Sort `write_addresses` first (or use try-lock + back-off, which the reference impl does).
- Forgetting `std::this_thread::yield()` in the retry path — under contention the loop pegs CPU and starves other threads.

---

## 5. Pitfalls catalogue

Numbered list, grouped by topic. These are the recurrent C++ concurrency mistakes; if you've debugged something that looks like one of these, the lesson is here.

### Memory model

1. **Mixed-mode access on `bool`/any non-atomic type is UB.** Plain `bool closed_` written under mutex but read outside → use `std::atomic<bool>`. TSan catches it; testing won't.
2. **`std::atomic` gives per-op atomicity only.** Compound ops like `--rc; if (rc == 0) sem.release()` race even with atomic `rc`. Use mutex around the **whole sequence** or the return-value pattern (`if (rc.fetch_sub(1) == 1) ...`).

### Condvar

3. **Inverted predicate.** `wait(lk, [&]{ return closed_; })` blocks while the predicate is false — sleeps forever on a healthy buffer. Predicate answers *"once true, stop waiting"*.
4. **Empty lambda capture `[]`.** Inside a member function, `queue_` is `this->queue_`. Use `[this]` or `[&]`.
5. **Don't charge ahead after wake.** Predicate has both progress and give-up disjuncts; check WHICH after waking.
6. **`return false` from `optional<T>`.** Use `std::nullopt`.
7. **Mutate predicate state OUTSIDE the lock.** Lost wakeup — `cv.notify_*` has no memory; the unlock-and-park atomicity is only w.r.t. holders of the *same* mutex.
8. **No `return` in lambda predicate.** `cv.wait` requires bool-convertible — bare expression statement is `void`.

### Mutex / locking

9. **`std::mutex::lock()` without matching `unlock()`.** Plain `{}` braces don't release. Use `lock_guard`/`unique_lock`/`scoped_lock`, or write manual `unlock()` (in reverse-of-acquire order).
10. **Single global `std::mutex` for "the resource."** Identifies the wrong granularity. Per-chopstick mutex lets non-adjacent philosophers eat simultaneously. **Match one mutex to one logical resource.**
11. **Locks declared INSIDE a function.** Stack-local → each call gets private locks → no mutual exclusion. **Sync primitives only work as shared objects across threads.** Promote to namespace/class scope.

### Semaphore

12. **`acquire()` where you meant `release()`.** Read aloud: "acquire" = wait, "release" = signal.
13. **`std::counting_semaphore<1>` then `release(n)` for n > 1.** UB. Use `counting_semaphore<N>` with `N` ≥ the broadcast size.
14. **Single shared `N_enough` for both directions of producer-consumer.** Sizing is per-direction. Collapsing too small deadlocks.
15. **No re-release on the closed-bail path.** Permits leak on shutdown.
16. **`std::counting_semaphore` has NO FIFO guarantee.** Spec: "at least one thread unblocks." Algorithms relying on FIFO (turnstile) leak ~15% timeouts under stress.

### Barrier

17. **`std::barrier<> b;`** — no default constructor; needs count.
18. **Domino phase 2 missing the `t1.acquire()` drain.** Leftover token leaks into round R+1; barrier broken.
19. **`bool ready_` instead of generation counter.** Round-tagging confusion → one-lap-ahead bug.

### Threads

20. **Non-joined non-detached `std::thread` destroys → `std::terminate`.** Always `.join()` before scope exit.
21. **`std::thread t([](){ /* uses captured ref to local */ });`** then the local goes out of scope before `t.join()` runs. Lifetime bug. Either join before scope exit or move ownership in.

### Other

22. **Tanenbaum `==` for `=` (assignment vs comparison).** Comparison-as-statement; result discarded → total deadlock on first meal. `-Wall -Wextra -Wunused-comparison` catches it.
23. **`sleep(1)` (POSIX, 1 SECOND) instead of `std::this_thread::sleep_for(1ms)`.** Wrong unit, wrong API.
24. **`std::queue::front()` on empty queue.** UB. Always check `empty()` first.
25. **Operators on smart-pointer-like types.** Auto-deref fires for methods/field access only — NOT operators. (Less of an issue in raw C++ than Rust, but `*it == X` for a raw iterator requires deref.)

---

## 6. Build / run / debug

```bash
# Regular build (CMake project)
cmake -S . -B build && cmake --build build
./build/<problem>/<binary>

# ThreadSanitizer — use during debugging
cmake -S . -B build-tsan -DSANITIZER=thread && cmake --build build-tsan
./build-tsan/<problem>/<binary>

# AddressSanitizer (mutually exclusive with TSan)
cmake -S . -B build-asan -DSANITIZER=address && cmake --build build-asan

# Stress loop — "is it really fixed?" test
for i in {1..1000}; do ./build-tsan/<problem>/<binary> || break; done

# GDB for deadlocks
gdb ./<binary>
(gdb) run
# Ctrl-C when it hangs
(gdb) thread apply all bt
# Look for threads stuck in pthread_cond_wait or pthread_mutex_lock
```

**Compiler flags worth turning on under stress.**
- `-Wall -Wextra` catches `==` vs `=` typos and unused-comparison warnings.
- `-fsanitize=thread` for TSan; `-fsanitize=address` for ASan; `-fsanitize=undefined` for UBSan.
- `-fno-omit-frame-pointer -g` so sanitizer output has symbols.

**Bar for "done."** 1000+ stress runs + sanitizer clean. Not "passed once."

**Limitations.** TSan catches **structural** problems (data race, lock-order-inversion). It cannot reason about runtime invariants (footman cap of N-1 prevents the cycle from closing). Conversely, an algorithm can pass TSan and still flake on liveness — the no-starve turnstile passes TSan but fails ~15% under stress because `std::counting_semaphore` doesn't promise FIFO.

---

## 7. Decision matrix — which primitive when

| Need | Reach for | Why |
|---|---|---|
| Serialise one resource | `std::mutex` + `std::lock_guard` | Cheap; RAII on the guard. |
| Multi-mutex acquire (deadlock-safe) | `std::scoped_lock{a, b}` | Variadic form does internal try-and-back-off. |
| Wait-on-predicate | `std::mutex` + `std::condition_variable` + `cv.wait(lk, pred)` | Direct expression of the wait condition; predicate overload handles spurious wakeups. |
| Resource pool / rate limit (count) | `std::counting_semaphore<N>` | Counts naturally. Watch the `<N>` template arg if you need `release(M)` for M > 1. |
| Reader-writer | `std::shared_mutex` (baseline); hand-rolled lightswitch + turnstile (illustrative) | Built-in is heavily optimised; hand-rolled is for showing the gradient (lightswitch → turnstile). |
| Phase synchronization | `std::barrier<>` (slide 22) **or** `mutex + cv + generation` (production-grade) | Generation-counter pattern is the production shape and what you'd write if asked. |
| Signal-with-memory (signal might fire before wait registers) | `std::binary_semaphore` | Has memory, unlike cv. Use for per-waiter wakeup queues. |
| Lock-free state transition on a single field | `std::atomic` with `compare_exchange_strong` | Single-shot test-and-set; no mutex needed. |
| Class-owned background loop | `std::thread` member + `std::atomic<bool>` stop flag + dtor join | The S4 shape. Disable copy/move when the class owns a thread. |
| Long-running worker that wakes on a coarse signal | `std::counting_semaphore` + recheck-stop-after-wake (I7) | Same wake fires for "real work" and "exit"; recheck disambiguates. |
| Reusable broadcast wake | `cv.notify_all()` | One line; substitute for `close(done)` in Go. |
| Per-thread done coordination | `std::thread::join()` on every thread, or a `std::counting_semaphore` countdown | C++ has no built-in WaitGroup; semaphore countdown is the closest. |

---

