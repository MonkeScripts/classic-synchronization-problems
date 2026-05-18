# CS3211 L9 — Classical Synchronization Problems: Per-Problem Exam Guide

> Audience: future-me sitting an exam where the questions are these exact problems (or close variants). Each section is **self-contained** — you should be able to answer an unseen question on that problem reading only its section + the cross-language patterns guide.
>
> The structure for every problem is:
>
> 1. **Problem statement** (what the question will frame).
> 2. **English-first algorithm** (always state this before you touch syntax).
> 3. **Invariants** (what you assert; what the oracle checks).
> 4. **Failure modes** (deadlock / livelock / starvation / lost wakeup / lost item).
> 5. **C++ approach(es) + mistakes** — predicate / semaphore / state-tracking variants.
> 6. **Go approach(es) + mistakes** — channel idioms, daemon, etc.
> 7. **Rust approach(es) + mistakes** — std vs tokio, RAII, `forget()`, async traps.
> 8. **Cross-language comparison table** — what changes, what doesn't.
> 9. **Exam answer template** — a skeleton you can adapt under time pressure.
>
> Each "mistake" is paired with the **diagnostic** (how you'd catch it) and the **lesson** (the one sentence you carry forward).

---

## Table of contents

0. [Triage tree — read this FIRST under exam pressure](#0-triage-tree--read-this-first-under-exam-pressure)
1. [Producer-Consumer](#1-producer-consumer)
2. [Readers-Writers](#2-readers-writers)
3. [Barrier](#3-barrier)
4. [Dining Philosophers](#4-dining-philosophers)
5. [Barbershop (Sleeping Barber)](#5-barbershop-sleeping-barber)
6. [H2O (Water Factory)](#6-h2o-water-factory)
7. [FIFO Semaphore](#7-fifo-semaphore)
8. [Search-Insert-Delete](#8-search-insert-delete)
9. [Bridge Crossing](#9-bridge-crossing)
10. [Universal cross-cutting lessons](#universal-cross-cutting-lessons)

---

## 0. Triage tree — read this FIRST under exam pressure

> Walk three trees in order: **(1)** what abstract pattern? → **(2)** which primitive in your target language? → **(3)** what cross-cutting axes still need to be decided? Then dive into the relevant per-problem section for the canonical implementation.

### Tree 1 — What pattern? (read the problem, walk the tree)

```
START: How do the threads/tasks RELATE?

├── (A) Sharing mutable state, coordinating access
│   ├── At most one at a time?                      → A1 EXCLUSION
│   ├── Up to N at a time (N>1)?                    → A2 BOUNDED-N
│   ├── Many readers OR one writer?                 → A3 READERS-WRITERS  (Per-Problem 2)
│   ├── 3+ asymmetric roles (e.g. reader/inserter
│   │   compatible, deleter exclusive)?             → A4 N-ROLE LIGHTSWITCH (SID §8, Bridge §9)
│   └── Wait until predicate over state is true?    → A5 PREDICATE-WAIT
│
├── (B) Passing data/messages between actors
│   ├── 1 → 1, single value, fire-and-forget?       → B1 ONESHOT
│   ├── Many → 1, queued?                            → B2 MPSC          (Per-Problem 1 producer-consumer)
│   ├── Many → many, work-stealing pool?             → B3 MPMC (mpsc + shared receiver)
│   ├── 1 → many, every event?                      → B4 BROADCAST
│   ├── 1 → many, latest snapshot only?             → B5 WATCH
│   └── Producer waits for consumer to finish?      → B6 REQUEST-RESPONSE  (barbershop, multi-doctor clinic)
│
├── (C) All actors must reach a sync point
│   ├── Once, then never again?                     → C1 LATCH (sync.WaitGroup, std::latch)
│   └── Reusable across phases?                     → C2 BARRIER  (Per-Problem 3)
│
└── (D) One actor orchestrates a protocol
    ├── Workers submit themselves, daemon collects? → D1 SUBMIT-YOURSELF DAEMON  (H2O §6)
    └── Driver picks who works each round?           → D2 ADDRESSED-WORKER DAEMON  (Smokers — Tier 2 Q10)
```

### Tree 2 — What primitive? (pattern × language)

| Pattern | C++ | Rust (sync) | Rust (tokio) | Go |
|---|---|---|---|---|
| **A1** Exclusion | `std::mutex` + RAII | `Arc<Mutex<T>>` | `Arc<tokio::sync::Mutex<T>>` (only if held across `.await`; else std) | `sync.Mutex` *or* `chan struct{}` cap 1 |
| **A2** Bounded-N | `std::counting_semaphore<N>` | `Arc<Semaphore>` (parking_lot) | `tokio::sync::Semaphore` | `chan struct{}` cap N |
| **A3** RW | `std::shared_mutex` (or hand-rolled lightswitch) | `RwLock` | `tokio::sync::RwLock` | `sync.RWMutex` |
| **A4** N-role | mutex + counter + N×binary semaphores (Patterns C11) | same shape | same | mutex + cond + state struct |
| **A5** Predicate-wait | `cv.wait(lk, pred)` | `cv.wait_while(g, !pred)` | `Notify` + Mutex re-check, OR `tokio::sync::Mutex` + manual loop | `for !pred { cv.Wait() }` |
| **B1** Oneshot | hand-rolled (promise/future) | `std::sync::mpsc` (cap 1) | `tokio::sync::oneshot` | `make(chan T)` rendezvous |
| **B2** MPSC | hand-rolled | `std::sync::mpsc` | `tokio::sync::mpsc` | `make(chan T, N)` |
| **B3** MPMC | hand-rolled / 3rd party | `crossbeam-channel` *or* `Arc<Mutex<Receiver>>` | `Arc<tokio::sync::Mutex<Receiver>>` (Patterns R13) | `make(chan T, N)` (channels are natively MPMC) |
| **B4** Broadcast | hand-rolled (cv+vector) | hand-rolled | `tokio::sync::broadcast` (R15) | `close(done)` for cancel; for events: hand-rolled |
| **B5** Watch | n/a | hand-rolled | `tokio::sync::watch` (R23) | hand-rolled |
| **B6** Request-response | hand-rolled | hand-rolled | `mpsc::<oneshot::Sender<()>>` (R16, R19) | `chan chan T` (G13 daemon) |
| **C1** Latch | `std::latch` (C++20) | `Arc<Once>` for once-flag; otherwise hand-rolled | `tokio::sync::Notify` + atomic | `sync.WaitGroup` |
| **C2** Barrier | `std::barrier` (C++20) | hand-rolled (cond+gen) | `tokio::sync::Barrier` | hand-rolled (G12) |
| **D1** Submit-yourself daemon | n/a | n/a | `mpsc<Request{go,done}>` (R20) | `chan chan T` (G13) |
| **D2** Addressed-worker daemon | n/a | n/a | N × `mpsc<()>` + shared `mpsc done` (R19) | N × per-worker channels |

> Cross-references: pattern IDs (R5, R13, etc.) point to `IMPLEMENTATION_PATTERNS.md`. Per-Problem § numbers point to sections of this file.

### Tree 3 — Orthogonal axes you must still decide

```
1. BLOCKING SEMANTIC on contention
   ├── Block forever            (default — `lock()`, `acquire()`, `send().await`)
   ├── Try once, fail fast       (`try_lock`, `try_acquire`, `try_send`)
   ├── Wait with timeout         (`try_lock_for`, `tokio::time::timeout`)
   ├── Balk = leave forever      (Sushi VIP, multi-doctor patient — problem says "leaves" / "returns error")
   └── Lossy = drop old          (broadcast Lagged, watch overwrites)

2. FAIRNESS / starvation
   ├── Don't care                (default semaphores; "any" fairness OK)
   ├── FIFO required             (Go channels; tokio Sem; or hand-rolled queue R21)
   ├── Anti-starvation needed    (turnstile pattern — RW §2.5, Bridge §9, SID §8.5)
   └── Symmetric handoff         (Bridge: drain → flip direction)

3. SHUTDOWN signal
   ├── Drop last sender          (channels in Rust/Go — closes naturally)
   ├── Close-as-broadcast        (Go `close(done)`, idiomatic for cancel)
   ├── Release N permits         (semaphore: `release(N_WAITERS)` per side; self-heal on bail)
   ├── Broadcast notify_all       (cv: `cv.notify_all()` + flag check)
   └── Sentinel value             (mpsc with sentinel item)

4. CONSISTENCY CONTRACT (what does each pair of compatible roles get?)
   ├── Strict serializable       (deleters in SID, writers in RW)
   ├── "May miss in-flight"      (searchers + inserters; readers + writers in eventual systems)
   ├── "See some snapshot"       (RW reads while writes happen — snapshot-of-the-moment)
   └── "Latest only"             (watch — intermediates dropped silently)
```

### Hot-keyword lookup (skim the problem statement first)

| Keyword in spec | Tells you... |
|---|---|
| "balks" / "leaves" / "returns error" / "queue full → walks away" | Tree 3 §1 → **Balk** |
| "may starve" / "starve-free" / "fair" / "in arrival order" | Tree 3 §2 → **FIFO / anti-starvation** |
| "drops oldest" / "limited buffer" / "may miss" | Tree 3 §1 → **Lossy** |
| "until [event]" / "with timeout" / "after Xms" | Tree 3 §1 → **Timeout** |
| "exclusive" / "no concurrent" / "must wait for" | Tree 1 → **A1** or **A4** (forbidden role pair) |
| "parallel" / "concurrent" / "many at once" | Tree 1 → **A2** or **A3** (allowed role pair) |
| "subscribe" / "publish" / "stream of events" | Tree 1 → **B4** broadcast |
| "latest" / "current" / "snapshot" | Tree 1 → **B5** watch |
| "waits for [other actor] to be done" | Tree 1 → **B6** request-response |
| "all reach point X before any proceeds" | Tree 1 → **C2** barrier |
| "phase" / "round" / "iteration" | Tree 1 → **C2** barrier (cycle-aware) |
| "agent" / "manager" / "coordinator" | Tree 1 → **D1** or **D2** daemon |

### How to use this section under exam pressure

1. **Read the problem once.** Don't start designing yet.
2. **Skim for hot-keywords** — they pre-classify into Tree 1 branches.
3. **Walk Tree 1**, forced choice at each branch — it picks A1..D2.
4. **Look up Tree 2** for your target language → primitive(s).
5. **Walk Tree 3** for each of the four orthogonal axes; default is `block / no-fairness / drop-last-sender / strictest contract`. Anything else needs an explicit clue from the problem.
6. **Now** dive into the relevant per-problem section (1..9 below) for canonical implementation, mistake catalogue, and exam-answer template.

If a problem doesn't fit cleanly into one branch, it's probably a *combination* — e.g. multi-doctor clinic = **B6** (request-response) layered over **B3** (MPMC mpsc + shared receiver). Compositions are common; identify each layer.

---

## 1. Producer-Consumer

### 1.1 Problem statement

N producers push items into a bounded buffer of capacity K. M consumers pop items from the buffer. Producers wait when full, consumers wait when empty. A `close()` operation signals shutdown — producers should bail; consumers should drain remaining items, then bail.

Concrete scenarios you may be asked about: log aggregator (workers → file flusher), web scraper (fetchers → parsers, parsers feed back → producers), thumbnail generator (file watchers → CPU-bound generators).

### 1.2 English-first algorithm

> A bounded queue. `push` waits **until** space is available **or** the buffer is closed. `pop` waits **until** an item is available **or** the buffer is closed-and-empty.
>
> The asymmetric bail is the key: `push` gives up the moment closed; `pop` only gives up when closed AND empty (drain remaining items first).
>
> After making progress, notify the **opposite** waiter set (after push, wake a popper; after pop, wake a pusher).

### 1.3 Invariants

- `0 <= queue.size() <= capacity` always.
- After a successful `push`, `queue.size() > 0`.
- After a successful `pop`, the item returned was at the front prior.
- `Produced == Consumed` once everyone has joined and the channel/queue is drained.
- Lost-item detector: print `Produced=X Consumed=Y Expected=Z`. They must agree.

### 1.4 Failure modes

| Mode | Triggered by |
|---|---|
| Lost wakeup | Notifying without holding the same lock as the waiter, or no waiter parked yet (with a primitive that has no memory — cv). |
| Lost item | `pop` bails on `closed` alone instead of `closed && empty`. |
| Spurious wakeup | Using `if` instead of `while`/predicate around `wait`. |
| Stolen wakeup | Newcomer steals the resource the woken waiter expected to find. Predicate-loop catches it. |
| Hang on shutdown | `close()` only flips a flag, doesn't `notify_all` (cv) or `release(N)` permits per side (semaphore). |
| Panic on shutdown (Go) | Closing a channel twice, or sending after close. |

### 1.5 C++ — two variants

#### A. Condvar variant: `std::mutex` + 2× `std::condition_variable`

```cpp
template <typename T>
class BoundedBuffer {
public:
    explicit BoundedBuffer(std::size_t capacity) : capacity_(capacity) {}

    bool push(T item) {
        std::unique_lock<std::mutex> lk{mut_};
        not_full_.wait(lk, [this] {
            return queue_.size() < capacity_ || closed_;
        });
        if (closed_) return false;                  // give-up disjunct
        queue_.push_back(std::move(item));
        not_empty_.notify_one();
        return true;
    }

    std::optional<T> pop() {
        std::unique_lock<std::mutex> lk{mut_};
        not_empty_.wait(lk, [this] {
            return !queue_.empty() || closed_;
        });
        if (closed_ && queue_.empty()) return std::nullopt;   // ASYMMETRIC bail
        T item = std::move(queue_.front());
        queue_.pop_front();
        not_full_.notify_one();
        return item;
    }

    void close() {
        std::scoped_lock lock(mut_);
        closed_ = true;
        not_full_.notify_all();
        not_empty_.notify_all();
    }

private:
    std::size_t capacity_;
    std::deque<T> queue_;
    std::mutex mut_;
    std::condition_variable not_full_, not_empty_;
    bool closed_ = false;     // protected by mut_
};
```

**Mistakes / lessons:**

| # | Mistake | Diagnostic | Lesson |
|---|---|---|---|
| 1 | "If buffer full, just return false" | CPU pegged, consumer starves | Polling burns CPU and starves the lock. Use condvar to release-and-park atomically. |
| 2 | Inverted predicate `wait(lk, [] { return closed_; })` | Producer hangs on healthy buffer | Predicate answers *"once true, I can stop waiting"* — not *"makes me wait"*. |
| 3 | Empty lambda capture `[]` | Compile error: undeclared identifier | Inside a method, `queue_` is `this->queue_`; lambdas need `[this]`. |
| 4 | After waking, charge ahead and push even if `closed_` | Items pushed into closed buffer, returns true | Predicate has TWO disjuncts; check WHICH after wake. |
| 5 | `pop` bails on `closed_` alone | `Consumed < Produced` | Asymmetric bail: pop drains first; only nullopt when `closed_ && queue.empty()`. |
| 6 | `return false` from `optional<T>` | Compile error | Use `std::nullopt`. |

#### B. Semaphore variant: `std::counting_semaphore<>` × 2 + `std::mutex`

```cpp
constexpr int MAX_PRODUCERS_WAITERS = NUM_PRODUCERS;
constexpr int MAX_CONSUMERS_WAITERS = NUM_CONSUMERS;

template <typename T>
class BoundedBuffer {
public:
    explicit BoundedBuffer(std::size_t capacity)
      : capacity_(capacity),
        spaces_sem_(static_cast<std::ptrdiff_t>(capacity)),
        items_sem_(0) {}

    bool push(T item) {
        spaces_sem_.acquire();
        if (closed_) { spaces_sem_.release(); return false; }   // self-heal
        { std::unique_lock<std::mutex> lk{mut_}; queue_.push_back(std::move(item)); }
        items_sem_.release();
        return true;
    }

    std::optional<T> pop() {
        items_sem_.acquire();
        std::unique_lock<std::mutex> lk{mut_};
        if (closed_ && queue_.empty()) {
            items_sem_.release();                                // self-heal
            return std::nullopt;
        }
        T item = std::move(queue_.front()); queue_.pop_front();
        spaces_sem_.release();
        return item;
    }

    void close() {
        { std::scoped_lock lk(mut_); closed_ = true; }
        spaces_sem_.release(MAX_PRODUCERS_WAITERS);
        items_sem_.release(MAX_CONSUMERS_WAITERS);
    }

private:
    std::size_t capacity_;
    std::deque<T> queue_;
    std::mutex mut_;
    std::counting_semaphore<> spaces_sem_;
    std::counting_semaphore<> items_sem_;
    std::atomic<bool> closed_{false};         // ATOMIC (read outside lock)
};
```

**Mistakes / lessons:**

| # | Mistake | Lesson |
|---|---|---|
| 7 | `bool closed_` read outside the mutex | **Mixed-mode access on any type is UB.** TSan catches it; testing won't. → `std::atomic<bool>`. |
| 8 | `close()` only sets the flag | **Semaphores have NO broadcast.** Parked waiters won't wake on a flag flip — they wake on a permit. Must `release(N)` per side. |
| 9 | `acquire()` where I meant `release()` | Read aloud: "acquire" = wait, "release" = signal. `close()`'s job is signal → release. |
| 10 | One `N_enough` shared by both semaphores | Sizing is **per-direction**: `MAX_PRODUCERS_WAITERS != MAX_CONSUMERS_WAITERS`. Collapsing with `max()` over-releases harmlessly; collapsing too small deadlocks. |
| 11 | No re-release on the closed-bail path | Consumed permit is leaked on shutdown. The **self-healing release-on-bail** is mandatory: over-release is harmless (recycled), missing release deadlocks. |

### 1.6 Go — channel variant

```go
const (
    BufferSize       = 8
    NumProducers     = 3
    NumConsumers     = 2
    ItemsPerProducer = 50
)

func main() {
    items := make(chan string, BufferSize)
    var produced, consumed atomic.Int32
    var pwg, cwg sync.WaitGroup

    for p := 0; p < NumProducers; p++ {
        pwg.Add(1)
        go func(pid int) {
            defer pwg.Done()
            for i := 0; i < ItemsPerProducer; i++ {
                items <- fmt.Sprintf("P%d#%d", pid, i)   // blocks if full
                produced.Add(1)                          // counter is YOUR job
            }
        }(p)
    }

    for c := 0; c < NumConsumers; c++ {
        cwg.Add(1)
        go func() {
            defer cwg.Done()
            for range items {                            // drains until closed
                consumed.Add(1)
            }
        }()
    }

    // Closer goroutine — exactly one site closes, after every producer finishes.
    go func() { pwg.Wait(); close(items) }()

    cwg.Wait()
    fmt.Printf("Produced=%d Consumed=%d Expected=%d\n",
        produced.Load(), consumed.Load(), NumProducers*ItemsPerProducer)
}
```

**Key insight:** **a channel IS a bounded buffer.** `make(chan T, N)`: send blocks if full, recv blocks if empty. `range ch` drains until `close`. The whole condvar/semaphore apparatus collapses.

**Mistakes / lessons:**

| # | Mistake | Lesson |
|---|---|---|
| 1 | `select { case x := <-items: ... }` infinite-loops after close | After close, recv is non-blocking and returns zero value. Use `for range items` for "drain until close". |
| 2 | `go func() { ... }` (no parens) | `go EXPR` requires a **call**, not a definition. Always `go func(){...}()`. |
| 3 | `for item := range items` with `item` unused | Go's unused-variable rule is hard error. Use `for range items`. |
| 4 | Forgetting `produced.Add(1)` | **Counters live OUTSIDE the primitive.** Channels synchronize, not observe. |
| 5 | Closing from a producer | Race: another producer might still be sending → `panic: send on closed channel`. |
| 6 | Closing twice | `panic: close of closed channel`. |
| 7 | Not closing at all | Consumers `range` forever → `cwg.Wait()` hangs. |

**Closer-goroutine pattern (canonical):** A dedicated goroutine waits on the producers' `WaitGroup` and is the only site that calls `close(ch)`. Senders drop their reference simply by exiting; the closer fires after `Wait()` proves everyone is past their last send.

**Rust-mpsc variant** is conceptually similar — `drop(tx)` is the substitute for `close()` because `mpsc` channels close when the last sender drops.

### 1.7 Rust — two variants

#### A. Condvar variant (almost a 1:1 C++ port)

```rust
struct BufferInner<T> {
    queue: VecDeque<T>,
    closed: bool,                 // lives BEHIND the lock; no `self.closed`
}

pub struct Buffer<T> {
    inner: Mutex<BufferInner<T>>,
    not_full: Condvar,
    not_empty: Condvar,
    capacity: usize,
}

impl<T> Buffer<T> {
    pub fn push(&self, item: T) -> Result<(), T> {
        let mut inner = self.inner.lock().unwrap();
        while !(inner.queue.len() < self.capacity || inner.closed) {
            inner = self.not_full.wait(inner).unwrap();   // returns LockResult<MutexGuard>
        }
        if inner.closed { return Err(item); }              // give back item, not bool
        inner.queue.push_back(item);
        self.not_empty.notify_one();
        Ok(())
    }

    pub fn pop(&self) -> Option<T> {
        let mut inner = self.inner.lock().unwrap();
        while inner.queue.is_empty() && !inner.closed {
            inner = self.not_empty.wait(inner).unwrap();
        }
        if inner.closed && inner.queue.is_empty() { return None; }
        let item = inner.queue.pop_front().unwrap();
        self.not_full.notify_one();
        Some(item)
    }

    pub fn close(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.closed = true;
        self.not_full.notify_all();
        self.not_empty.notify_all();
    }
}
```

**Equivalent to `wait_while(guard, |s| !pred)`:** Rust offers a predicate overload too, but the polarity is **opposite** to C++: `wait_while(g, |s| pred)` waits *while* `pred` is true; C++ `wait(lock, pred)` waits *until* `pred` is true. Negate the body when porting.

**Mistakes / lessons (Rust syntax leaks, not algorithm):**

| # | Mistake | Lesson |
|---|---|---|
| 1 | `std::move(item)` | Rust moves implicitly — by-value parameters already transfer ownership. |
| 2 | `Ok()` | `Result<(), T>::Ok` carries a value; unit is `Ok(())`. |
| 3 | `if cond { Err(item) }` then fall-through | An `if` with no `else` evaluates to `()`. Either `return Err(item);` or wrap rest in `else`. |
| 4 | `self.closed` | State lives behind the lock — `inner.closed`, never `self.closed`. |
| 5 | `Ok(closed)` on bail | Wrong direction. Closed → `Err(item)`, returning the unconsumed item. |
| 6 | `wait(inner).lock().unwrap()` | `Condvar::wait(guard)` returns `LockResult<MutexGuard>` directly — don't re-lock. |
| 7 | `.empty()` on `VecDeque` | Rust naming: `is_empty`, `is_some`, `is_none`. |
| 8 | `inner.pop_front()` (where `inner` is the guard) | `pop_front` is on `inner.queue`, not on the guard. (Same trap on `inner.push_back`.) |
| 9 | Returning `item` after `match item { Ok(s) => ... }` | `match` consumes the scrutinee; only `s` survives in the arm. |

#### B. mpsc variant — `std::sync::mpsc::sync_channel(N)`

```rust
let (tx, rx) = sync_channel::<String>(BUFFER_SIZE);

// producers — clone tx, share via move
for pid in 0..NUM_PRODUCERS {
    let tx = tx.clone();
    let produced = Arc::clone(&produced);
    thread::spawn(move || {
        for i in 0..ITEMS_PER_PRODUCER {
            tx.send(format!("P{}#{}", pid, i)).unwrap();
            produced.fetch_add(1, Ordering::SeqCst);
        }
    });
}
drop(tx);                                                // PITFALL A — close the channel

// consumers — Receiver is NOT Clone, share via Arc<Mutex<Receiver<T>>>
let recv_lock = Arc::new(Mutex::new(rx));
for _ in 0..NUM_CONSUMERS {
    let consumed = Arc::clone(&consumed);
    let recv_lock = Arc::clone(&recv_lock);
    thread::spawn(move || loop {
        let item = {                                     // PITFALL B — tight lock scope
            let rx = recv_lock.lock().unwrap();
            rx.recv()
        };  // guard dropped HERE, before match
        match item {
            Ok(s) => { std::hint::black_box(s); consumed.fetch_add(1, Ordering::SeqCst); }
            Err(_) => break,                             // channel closed
        }
    });
}
```

**Two pitfalls — every Rust mpsc question hits these:**

- **(A) `drop(tx)` in main.** `mpsc` channels close only when **all** senders drop. Main holds the original `tx` until you explicitly `drop(tx)`. Without it, `rx.recv()` never returns `Err` → consumers deadlock.
- **(B) `Arc<Mutex<Receiver<T>>>` for multi-consumer.** `Receiver` is `Send` but **not `Sync`** and not `Clone` — the only safe way two consumer threads share it is behind a Mutex. **Tight-scope the lock** so the recv `.await` (or in std-mpsc the blocking recv) doesn't serialize the *waiting* on top of the dequeue.

Performance caveat: `std::sync::mpsc` with `Arc<Mutex<Receiver>>` is *effectively serial*. `crossbeam-channel`'s `Receiver` is `Clone` and `Sync` and gives you true concurrent multi-consumer.

### 1.8 Cross-language comparison

| Aspect | C++ Condvar | C++ Semaphore | Go Channel | Rust mpsc |
|---|---|---|---|---|
| Lines of sync logic | ~90 | ~100 | ~20 | ~40 |
| Bounded buffer expression | `std::deque` + capacity field | semaphore counter | `make(chan T, N)` (literal) | `sync_channel(N)` |
| Wait-while-full | `not_full_.wait(lk, pred)` | `spaces_sem_.acquire()` | `ch <- x` | `tx.send(x)` |
| Wait-while-empty | `not_empty_.wait(lk, pred)` | `items_sem_.acquire()` | `<-ch` | `rx.recv()` |
| Spurious wakeups | Yes (predicate loop) | No | No | No |
| Shutdown LOC | 1 line `notify_all` × 2 | `release(N)` per side + self-heal | `close(ch)` | `drop(tx)` |
| Shutdown failure | Hang (silent) | Hang or starve (silent) | **Panic (loud)** if double-close or send-after-close | Hang if main forgets drop |
| Predicate-loop polarity | `wait(lk, pred)` "until true" | n/a | n/a | `wait_while(g, pred)` "while true" — **opposite of C++** |

### 1.9 Exam answer template

> **State the algorithm in English first.** Don't touch syntax until you can say "wait until <X> or <closed>".
>
> 1. Define the bounded queue + closed flag.
> 2. Choose primitive: condvar (predicate state explicit), semaphore (resource counts natural), or channel (Go's literal `make(chan T, N)`).
> 3. Push waits-until `space || closed`; bails on `closed` alone.
> 4. Pop waits-until `non-empty || closed`; bails ONLY on `closed && empty` (asymmetric).
> 5. After progress, notify the **opposite** condition.
> 6. Close: cv → `notify_all` × 2; semaphore → `release(MAX_WAITERS)` per side + self-heal on bail; channel → dedicated closer goroutine.

---

## 2. Readers-Writers

### 2.1 Problem statement

Many readers OR one writer at a time over a shared data structure (typical scenario: KV cache, config service, order book). Three increasingly sophisticated solutions:

1. **Built-in RW lock** — `std::shared_mutex`, `tokio::sync::RwLock`, `sync.RWMutex`.
2. **Lightswitch (Downey)** — hand-rolled, **reader-preference**, starves writers.
3. **No-starve turnstile** — adds a third semaphore so writers can't be indefinitely starved.

### 2.2 English-first algorithm

#### Lightswitch (slide 10–11)
```
bibo (binary mutex) protects rc (reader count)
roomEmpty: held by reader collective OR by writer
turnstile: not present in Lightswitch

Reader:
    bibo.acquire
    rc++; if rc == 1: roomEmpty.acquire   // first reader locks out writers
    bibo.release
    READ
    bibo.acquire
    rc--; if rc == 0: roomEmpty.release   // last reader yields
    bibo.release

Writer:
    roomEmpty.acquire
    WRITE
    roomEmpty.release
```

#### No-starve turnstile (slide 12)
```
turnstile: gate every entrant must pass through
Writers HOLD turnstile from entry to exit (closes the gate behind them)
Readers gate-pass: acquire then release immediately

Reader:
    turnstile.acquire; turnstile.release    // gate-pass
    bibo.acquire; rc++; if rc == 1: roomEmpty.acquire; bibo.release
    READ
    bibo.acquire; rc--; if rc == 0: roomEmpty.release; bibo.release

Writer:
    turnstile.acquire
    roomEmpty.acquire
    WRITE
    roomEmpty.release
    turnstile.release
```

### 2.3 Invariants

- `readers > 0 → writers == 0`
- `writers > 0 → readers == 0 && writers == 1`

Implement these as an oracle **separate** from the algorithm's state. Do NOT use `rc` (algorithm) as the witness for the invariant — they differ during the "first-reader-blocked" window (rc==1 but no thread has actually entered the room yet).

### 2.4 Failure modes

| Mode | Triggered by |
|---|---|
| Compound-op race | `--rc; if (rc == 0) sem.release()` is THREE ops; even with atomic rc, two threads can both observe 0 and both release → UB / over-send. |
| Writer starvation | Plain Lightswitch: as long as readers keep arriving faster than `rc → 0`, writers wait forever. |
| Liveness flake | Turnstile algorithm + a primitive that doesn't promise FIFO wakeup → ~85% pass rate (the flagship cross-language finding). |

### 2.5 C++ — three implementations

#### A. `std::shared_mutex` baseline

```cpp
class KVCacheShared {
    std::shared_mutex mu_;
    std::unordered_map<std::string,std::string> map_;
    ActiveCounters counters_;
public:
    std::string get(const std::string& k) {
        std::shared_lock lk(mu_);
        counters_.enter_read();
        auto it = map_.find(k);
        std::string r = it != map_.end() ? it->second : "";
        counters_.exit_read();
        return r;
    }
    void set(const std::string& k, const std::string& v) {
        std::unique_lock lk(mu_);
        counters_.enter_write();
        map_[k] = v;
        counters_.exit_write();
    }
};
```

#### B. Hand-rolled Lightswitch

```cpp
class KVCacheHandRolled {
    std::counting_semaphore<1> bibo_sem_{1};
    std::counting_semaphore<1> roomEmptySem_{1};
    int rc_ = 0;                                          // PLAIN int — guarded by bibo_
    std::unordered_map<std::string,std::string> map_;
    ActiveCounters counters_;
public:
    std::string get(const std::string& k) {
        bibo_sem_.acquire();
        ++rc_;
        if (rc_ == 1) roomEmptySem_.acquire();
        bibo_sem_.release();
        counters_.enter_read();                            // OUTSIDE bibo (post-exclusion)
        auto it = map_.find(k);
        std::string r = it != map_.end() ? it->second : "";
        counters_.exit_read();
        bibo_sem_.acquire();
        --rc_;
        if (rc_ == 0) roomEmptySem_.release();             // INSIDE bibo (compound atomicity)
        bibo_sem_.release();
        return r;
    }
    void set(const std::string& k, const std::string& v) {
        roomEmptySem_.acquire();
        counters_.enter_write();
        map_[k] = v;
        counters_.exit_write();
        roomEmptySem_.release();
    }
};
```

#### C. Turnstile (no-starve, in theory)

Add a third semaphore `turnstile_sem_{1}`. Writers `acquire` it before `roomEmpty.acquire` and `release` it at the very end (after `roomEmpty.release`). Readers `acquire`-then-immediately-`release` it as the very first action.

**Mistakes / lessons:**

| # | Mistake | Lesson |
|---|---|---|
| 1 | `counters_.enter_read()` placed BEFORE acquiring `roomEmpty` | Counters are an **oracle**, not solution state. Bump only AFTER you've actually secured exclusion. |
| 2 | Used `counters_.readers == 1` as "am I the first reader?" | Conflating instrumentation with algorithm. They differ during the "first-reader-blocked" window. **Maintain a separate `int rc_`.** |
| 3 | `int rc_;` without initializer | UB on first read. Always `int rc_ = 0;`. |
| 4 | `--rc_; if (rc_ == 0) sem.release();` outside the lock | Compound-op race. Atomic on `rc_` doesn't fix it — two threads both see 0, both release. Mutex must wrap **the whole sequence**. |
| 5 | "Made it `std::atomic`, problem solved" | **Per-op atomicity ≠ multi-op atomicity.** Atomics fix the *individual* `--rc_`, not the decrement-then-check sequence. Use `compare_exchange` or a mutex around the block. |
| 6 | `counters_.enter_read()` inside `bibo_sem_` | Safe but over-serialised. `counters_` are atomic; can hoist outside. |
| 7 | Turnstile passes the cold-run test but flakes ~15% under stress | **Liveness ≠ correctness.** Downey's proof assumes fair scheduling. `std::counting_semaphore` doesn't promise FIFO. **Same code is bulletproof in Go and Rust, flaky in C++**, solely because of primitive fairness. |

### 2.6 Go — three implementations

```go
type KVCacheRW struct {                        // baseline: sync.RWMutex
    mu       sync.RWMutex
    data     map[string]string
    counters Counters
}

type KVCacheHandRolled struct {                // Lightswitch + turnstile
    data      map[string]string
    counters  Counters
    bibo      chan struct{}                    // cap 1, binary semaphore
    emptyRoom chan struct{}                    // cap 1
    turnstile chan struct{}                    // cap 1
    rc        int                              // PLAIN int — guarded by bibo
}

func NewKVCacheHandRolled() *KVCacheHandRolled {
    return &KVCacheHandRolled{
        data:      make(map[string]string),
        bibo:      make(chan struct{}, 1),     // *** cap 1, NOT cap 0 ***
        emptyRoom: make(chan struct{}, 1),
        turnstile: make(chan struct{}, 1),
    }
}

func (c *KVCacheHandRolled) Get(k string) string {
    c.turnstile <- struct{}{}; <-c.turnstile               // gate-pass
    c.bibo <- struct{}{}
    c.rc++
    if c.rc == 1 { c.emptyRoom <- struct{}{} }
    <-c.bibo
    c.counters.EnterRead()
    v := c.data[k]
    c.counters.ExitRead()
    c.bibo <- struct{}{}
    c.rc--
    if c.rc == 0 { <-c.emptyRoom }
    <-c.bibo
    return v
}

func (c *KVCacheHandRolled) Set(k, v string) {
    c.turnstile <- struct{}{}
    c.emptyRoom <- struct{}{}
    c.counters.EnterWrite(); c.data[k] = v; c.counters.ExitWrite()
    <-c.emptyRoom
    <-c.turnstile
}
```

**Two binary-semaphore patterns:**

| | Pattern A (textbook) | **Pattern B (Go-idiomatic)** |
|---|---|---|
| Initial state | seeded with one token | **empty** |
| Acquire | `<-ch` | `ch <- struct{}{}` |
| Release | `ch <- struct{}{}` | `<-ch` |
| Mental model | "here's a token; take it" | "I put myself in, I take myself out" |
| Scales to N? | no | **yes** — just `make(chan struct{}, N)` |

The Lightswitch + turnstile code uses Pattern B. It's the same shape Go programmers use everywhere for counting semaphores.

**Mistakes / lessons:**

| # | Mistake | Lesson |
|---|---|---|
| 1 | `make(chan struct{})` for binary semaphore | **cap 0 is RENDEZVOUS, not a semaphore.** Unbuffered send blocks until a paired recv. For a semaphore you need cap 1 (state lives in the buffer). |
| 2 | Field access without receiver: `bibo <- struct{}{}` | Always `c.bibo`. Go has no implicit `this`. |
| 3 | Trailing-comma rule in composite literals | Every line of a literal needs a trailing comma. |
| 4 | Forgot to bench the second impl | Hygiene — always run all variants. |

### 2.7 Rust — async with tokio

```rust
use tokio::sync::Semaphore;
use std::sync::{Mutex, atomic::{AtomicI32, Ordering}};

pub struct KVCacheHandRolled {
    map: Mutex<HashMap<String, String>>,    // std::sync — short critical sections, no .await across
    counters: Counters,
    turnstile: Semaphore,                   // cap 1
    room_empty: Semaphore,                  // cap 1
    bibo: Semaphore,                        // cap 1
    rc: AtomicI32,                          // atomic only because Sync; mutated under bibo
}

#[async_trait]
impl Cache for KVCacheHandRolled {
    async fn get(&self, k: &str) -> String {
        drop(self.turnstile.acquire().await.unwrap());                  // gate-pass

        {
            let _bibo = self.bibo.acquire().await.unwrap();
            let new_rc = self.rc.fetch_add(1, Ordering::SeqCst) + 1;    // USE return value
            if new_rc == 1 {
                let permit = self.room_empty.acquire().await.unwrap();
                permit.forget();                                         // cross-task lifetime!
            }
        }

        self.counters.enter_read();
        let out = {
            let guard = self.map.lock().unwrap();                        // tight scope!
            guard.get(k).cloned().unwrap_or_default()
        };  // guard MUST be lexically dead before next .await (Send analysis)
        self.counters.exit_read();

        {
            let _bibo = self.bibo.acquire().await.unwrap();
            let new_rc = self.rc.fetch_sub(1, Ordering::SeqCst) - 1;
            if new_rc == 0 {
                self.room_empty.add_permits(1);                          // reciprocal of forget
            }
        }
        out
    }

    async fn set(&self, k: String, v: String) {
        let _turnstile = self.turnstile.acquire().await.unwrap();        // LIFO drop
        let _room      = self.room_empty.acquire().await.unwrap();       // → release in reverse
        self.counters.enter_write();
        { let mut g = self.map.lock().unwrap(); g.insert(k, v); }
        self.counters.exit_write();
    }
}
```

**The two non-obvious Rust+tokio idioms:**

1. **Permit RAII + `forget()` + `add_permits()`.** Tokio permits return on Drop. For "first reader acquires, last reader releases (different task)," call `permit.forget()` to break the RAII contract; the reciprocal task calls `add_permits(1)` to put a permit back. Both lifetimes match the algorithm's, not Rust's lexical scopes.
2. **`!Send` MutexGuard across `.await`.** `std::sync::MutexGuard` is `!Send`. Holding one across `.await` makes the whole future `!Send`, which `tokio::spawn` rejects. **Block-scope** the guard so it's lexically dead before the next `.await`. **`drop(guard)` does NOT help** — the analysis is lexical, not dataflow.

**Mistakes / lessons:**

| # | Mistake | Lesson |
|---|---|---|
| 1 | `.release()` on tokio Semaphore | Doesn't exist. Permits are RAII; release is implicit on Drop. |
| 2 | `.acquire()` without `.await` | Returns an unstarted Future; nothing happens. |
| 3 | `self.rc.fetch_add(1, ...); if self.rc == 1 { ... }` | Compound-op race + can't compare `AtomicI32` to `i32` directly. Use **fetch_add's return value**: `let new_rc = self.rc.fetch_add(1, Ord) + 1;`. |
| 4 | Forgot `permit.forget()` on first-reader | Permit auto-returns on scope exit → roomEmpty released while readers still in room → invariant fires. |
| 5 | `MutexGuard` held across `.await` | "Future is not Send" compile error. Tight-scope the guard. |
| 6 | `drop(guard); something.await;` | **Doesn't work.** Send analysis is lexical. Wrap in `{ ... }` block instead. |

### 2.8 The marquee cross-language finding (THE most important takeaway)

| Language | Primitive | FIFO wakeup? | Turnstile stress (500 runs) |
|---|---|---|---|
| C++ | `std::counting_semaphore<1>` | **No** ("at least one" thread unblocks) | ~85% pass, 15% timeout |
| Go | `chan struct{}` cap 1 | **Yes** (spec: served in FIFO order) | **500/500 clean** |
| Rust | `tokio::sync::Semaphore` | **Yes** (docs: FIFO) | **500/500 clean** |

**Same algorithm.** **Same benchmark.** Only the primitive's wakeup-fairness guarantee differs.

> **Correctness transfers across languages; liveness doesn't. Liveness is a joint property of the algorithm and the primitive's scheduler.**
>
> Downey's proof assumes "eventually a fair waiter wakes" — silently. Real primitives vary. Before claiming starve-free, check what your primitive promises about wakeup ordering.

### 2.9 Exam answer template

1. Three solutions in increasing sophistication: built-in RW lock → Lightswitch → turnstile.
2. Lightswitch is **reader-preference** and starves writers — say so explicitly.
3. Turnstile fixes starvation **assuming fair-wakeup primitive**.
4. Compound-op race is the #1 implementation pitfall: state both decrement and conditional-release must be atomic together.
5. Counters/oracle must be independent of algorithm state.
6. **Mention the FIFO-wakeup gap** — it's the marquee insight.

---

## 3. Barrier

### 3.1 Problem statement

N threads must all reach a synchronization point before any may proceed. Reusable across rounds. (Scenario: parallel iterative solver with phases; turn-based simulation; race start.)

### 3.2 English-first algorithm — three shapes

#### A. Domino (slide 20–21)

Two binary turnstiles. Last arriver opens entry; each thread acquires + immediately releases (the "domino"); phase 2 mirrors. Critical line in phase 2: drain the leftover token before reuse.

```
arrive_and_wait():
    lock; count++; if count == N: t2.acquire (close exit); t1.release (open entry); unlock
    t1.acquire; t1.release          // domino in
    lock; count--; if count == 0: t1.acquire (drain leftover); t2.release (open exit); unlock
    t2.acquire; t2.release          // domino out
```

State cycle: `(0, 0, 1) → (0, 0, 1)` per round.

#### B. Preloaded turnstile (slide 22)

Single counting semaphore, init 0. N-th arriver `release(N)`. Each thread `acquire()` exactly once. Reusable for free — N in, N out, count returns to 0.

Requires `counting_semaphore<N>` (max ≥ N), not `<1>`. `release(N)` on `counting_semaphore<1>` is **UB**.

#### C. Mutex + Condvar + generation counter (production-grade)

```cpp
class MyBarrierCond {
    std::mutex mu_;
    std::condition_variable cv_;
    std::size_t expected_, count_ = 0;
    std::uint64_t generation_ = 0;        // monotonic; tags rounds uniquely
public:
    void arrive_and_wait() {
        std::unique_lock<std::mutex> lock(mu_);
        const std::uint64_t gen = generation_;
        if (++count_ == expected_) {
            count_ = 0;
            ++generation_;
            cv_.notify_all();
            return;
        }
        cv_.wait(lock, [this, gen]{ return gen != generation_; });
    }
};
```

**Generation, not boolean:** a `bool ready_` can't tell round R from round R+1.

### 3.3 Failure modes

| Mode | Triggered by |
|---|---|
| One-lap-ahead | Single `bool` flag — round R thread sees `ready=true`, exits, but predicate can't tell from round R+1's `ready=true`. |
| Domino reuse breaks | Skipping the `t1.acquire()` drain in phase 2 — leftover token races into round 2. |
| `release(N)` on cap-1 semaphore | UB. Use `counting_semaphore<N>`. |
| `sync.WaitGroup` reuse panic (Go) | Reusing a `WaitGroup` for round 2 → panic *"WaitGroup is reused before previous Wait has returned"*. |

### 3.4 C++ — three variants in one file

```cpp
// Variant 1: domino (binary semaphore × 2).
// Variant 2: preloaded — counting_semaphore<N_THREADS> sem{0}; release(N) at last; one acquire per thread.
// Variant 3: cond + generation.
```

**Mistakes / lessons:**

| # | Mistake | Lesson |
|---|---|---|
| 1 | Phase-1 didn't `t2.acquire()` inside the lock | Threads race through phase 2 before last arriver opens phase 1. |
| 2 | Phase-2 didn't `t1.acquire()` drain | Leftover token leaks into round R+1; barrier broken. |
| 3 | `bool ready_` instead of generation counter | Round-tagging confusion → one-lap-ahead bug. |

### 3.5 Go — two variants

#### A. Two `*sync.WaitGroup`s with snapshot-and-swap (slide-23 shape)

```go
type MyBarrier struct {
    expected int
    mu       sync.Mutex
    entry_wg *sync.WaitGroup
    exit_wg  *sync.WaitGroup
}

func freshWG(n int) *sync.WaitGroup { wg := &sync.WaitGroup{}; wg.Add(n); return wg }

func NewMyBarrier(n int) *MyBarrier {
    return &MyBarrier{expected: n, entry_wg: freshWG(n), exit_wg: freshWG(n)}
}

func (b *MyBarrier) ArriveAndWait() {
    b.mu.Lock(); wg := b.entry_wg; b.mu.Unlock()
    wg.Done(); wg.Wait()
    b.mu.Lock(); if b.entry_wg == wg { b.entry_wg = freshWG(b.expected) }; b.mu.Unlock()

    b.mu.Lock(); wg2 := b.exit_wg; b.mu.Unlock()
    wg2.Done(); wg2.Wait()
    b.mu.Lock(); if b.exit_wg == wg2 { b.exit_wg = freshWG(b.expected) }; b.mu.Unlock()
}
```

#### B. `sync.Mutex` + `sync.Cond` + generation (the cleaner shape)

```go
type MyBarrier struct {
    expected, count int
    generation      uint64
    mu              sync.Mutex
    cv              *sync.Cond
}

func NewMyBarrier(n int) *MyBarrier {
    b := &MyBarrier{expected: n}
    b.cv = sync.NewCond(&b.mu)
    return b
}

func (b *MyBarrier) ArriveAndWait() {
    b.mu.Lock(); defer b.mu.Unlock()
    gen := b.generation
    b.count++
    if b.count == b.expected {
        b.count = 0
        b.generation++
        b.cv.Broadcast()
        return
    }
    for b.generation == gen { b.cv.Wait() }
}
```

**Mistakes / lessons:**

| # | Mistake | Lesson |
|---|---|---|
| 1 | `wg.Add(expected)` inside the struct literal | Composite literals only set fields to **values**; `Add` returns nothing. Use a constructor (`freshWG`). |
| 2 | Reusing one `sync.WaitGroup` (`Done; Wait; Add(1)`) | Panic: *"WaitGroup is reused before previous Wait has returned."* **`sync.WaitGroup` is a single-use latch, not a cyclic barrier.** |
| 3 | Fields as `sync.WaitGroup` (value) | Can't compare values; can't legally copy (`noCopy` lint). Need `*sync.WaitGroup`. |
| 4 | Copy-paste typo: `if b.exit_wg == wg` (should be `wg2`) | Symmetric two-phase code is a copy-paste hazard. The wrong snapshot variable means the swap never fires. |

### 3.6 Rust — two variants

#### A. `std::sync::Mutex` + `std::sync::Condvar` + generation

```rust
struct BarrierState { count: usize, generation: u64 }

pub struct MyBarrier { expected: usize, state: Mutex<BarrierState>, cv: Condvar }

impl MyBarrier {
    pub fn arrive_and_wait(&self) {
        let mut state = self.state.lock().unwrap();
        let gen = state.generation;
        state.count += 1;                                              // mutate THROUGH guard
        if state.count == self.expected {
            state.count = 0;
            state.generation += 1;
            self.cv.notify_all();
            return;
        }
        let _guard = self.cv
            .wait_while(state, |s| gen == s.generation)                // wait WHILE pred true
            .unwrap();                                                  // named binding (lint)
    }
}
```

#### B. `tokio::sync::Semaphore` preloaded turnstile

```rust
async fn arrive_and_wait(&self) {
    // ... last arriver: self.t1.add_permits(self.expected);
    self.t1.acquire().await.unwrap().forget();                          // .forget() MANDATORY
    // ... last arriver: self.t2.add_permits(self.expected);
    self.t2.acquire().await.unwrap().forget();
}
```

**Mistakes / lessons (this is the longest catalogue of any problem — useful as a Rust-idiom refresher):**

| # | Mistake | Lesson |
|---|---|---|
| 1 | `acquire().await()` (parens) | `.await` is **postfix syntax**, not a method. `expr.await`. |
| 2 | `acquire().await.forget()` no `.unwrap()` | `Semaphore::acquire` returns `Result` (closeable). |
| 3 | Missing `;` on `state.count -= 1` | Statements need terminators except last expression. Brace placement = grammar. |
| 4 | "The permit just dies" | **No.** Permit's Drop returns it via `add_permits(1)`. Without `.forget()`, gate never empties → round 2 races. |
| 5 | `self.state.count = 0` (Mutex itself) | Mutex doesn't expose `T`'s fields. Mutate **through the guard** only. |
| 6 | `let mut count = state.count; count += 1;` | `usize` is `Copy`. Local copy is independent. → `state.count += 1;`. |
| 7 | `wait_while(self.state.lock().unwrap(), ...)` while holding | `std::sync::Mutex` is **NOT reentrant**. `wait_while` takes the guard you already hold. |
| 8 | Predicate polarity: `gen != s.generation` | C++ `wait(lock, pred)` waits **until** true; Rust `wait_while(g, pred)` waits **while** true. **Inverse.** |
| 9 | Closure parameter named `gen` shadows outer | Closures shadow captures. Pick `s` for guard params. |
| 10 | `let _ = wait_while(...)` | Triggers `let_underscore_lock` lint — drops guard immediately. Use `let _guard = ...;` (named). |

### 3.7 Cross-language comparison

| Variant | Pros | Cons |
|---|---|---|
| Domino (slide 20–21) | Works with binary semaphores | Needs the drain line; easy to omit |
| Preloaded (slide 22) | Reusable for free, cleanest | Requires `counting_semaphore<N>` (or higher); `forget()` mandatory in Rust |
| Cond + generation | No semaphore at all; fewest moving parts | One extra `notify_all` per round; CV slower under heavy contention |

In production Rust/Go/C++: the cond + generation version is the cleanest. Semaphore versions are pedagogical.

### 3.8 Exam answer template

> **Three reusable barrier shapes:**
>
> 1. Two-turnstile domino (slide 20–21): close-exit-then-open-entry; remember to drain the leftover token in phase 2.
> 2. Preloaded counting semaphore (slide 22): `release(N)` once, each thread `acquire`s once. Reusable for free. Requires `counting_semaphore<N>`.
> 3. Mutex + condvar + **generation counter**: thread snapshots `gen` at entry, last arriver flips `gen++` and `notify_all`. Predicate-loop on `gen != snapshot` handles spurious wakeups.
>
> **Generation counter, not boolean:** a `bool ready` can't distinguish round R from R+1.
>
> **Language gotchas:** Go's `sync.WaitGroup` is single-use, not cyclic — use snapshot-and-swap with `*sync.WaitGroup` or skip to cond+gen. Rust's tokio preloaded variant **must** call `permit.forget()` or the gate refills on Drop.

---

## 4. Dining Philosophers

### 4.1 Problem statement

N philosophers around a table, N chopsticks between them. Each needs both neighbors' chopsticks to eat. The classical lecture demonstration of: **deadlock, livelock, starvation, and the difference between them.**

### 4.2 English-first failure modes

| Mode | What |
|---|---|
| **Deadlock** | Lock-acquire-graph cycle. Naive "everyone takes left first" → cycle `0→1→2→3→4→0` → hangs in seconds. |
| **Livelock** | No thread blocks, no thread progresses. Try-and-back-off in lockstep. |
| **Starvation** | Deadlock-free + livelock-free, but one thread is consistently late. Tanenbaum's classic version starves a hungry philosopher between two alternating eaters. |

### 4.3 The five strategies

1. **Naive** (left then right, every philosopher) — demonstrates deadlock; leave it in, annotated.
2. **Asymmetric / odd-even ring** — one or more philosophers reverse the order. Breaks the cycle **structurally**.
3. **`std::scoped_lock` / try-and-back-off** — library or hand-rolled. Tries to acquire all atomically; on partial failure, releases and retries (other order). Deadlock-free; livelock-prone but rare.
4. **Footman semaphore** — cap concurrent eaters at `N-1`. **Pigeonhole**: no full cycle can form. Deadlock-free at runtime, but the lock-acquire graph still has a cycle, so structural analyzers (TSan) flag it.
5. **Tanenbaum state-tracking** — no chopstick locks. Per-philosopher state under one mutex; per-philosopher CV (or semaphore). Neighbors `safe_to_eat` you when they finish. Deadlock-free, **starvation-prone**.

### 4.4 The marquee cross-language finding (TSan vs Go race)

| Strategy | C++ TSan | Go `-race` | Why |
|---|---|---|---|
| `std::scoped_lock` | clean | n/a | try-and-back-off internal — TSan recognizes |
| Footman | **lock-order-inversion warning** | n/a | Cycle in graph; cap is *runtime* prevention, invisible to TSan |
| Asymmetric (odd/even ring) | n/a | clean | Cycle absent in graph |
| Try-and-back-off | n/a | clean | Non-blocking try doesn't add a "hold-while-blocking" edge |

> **TSan can detect *structural* deadlock potential. It cannot reason about *quantitative* arguments like "at most N−1 → no full cycle possible."**
>
> Same lesson family as the readers-writers fairness gap: an analyzer is right about what it can see; what it can't see is the whole prevention argument.

### 4.5 C++ — three flavors implemented

```cpp
std::array<std::mutex, N> chopsticks;            // GLOBAL — must be shared!
std::counting_semaphore<> num_eaters{N - 1};    // GLOBAL — footman cap

void eat_scoped_lock(std::size_t pid) {
    auto left = pid, right = (pid + 1) % N;
    std::scoped_lock lk{chopsticks[left], chopsticks[right]};   // try-and-back-off internal
    // eat
}

void eat_footman(std::size_t pid) {
    auto left = pid, right = (pid + 1) % N;
    num_eaters.acquire();                        // pigeonhole cap
    chopsticks[left].lock();
    chopsticks[right].lock();
    // eat
    chopsticks[right].unlock();                  // reverse-of-acquire
    chopsticks[left].unlock();
    num_eaters.release();
}

// Tanenbaum: state machine, no chopstick locks at all
void tanenbaum_safe_to_eat(std::size_t pid) {    // PRECONDITION: holds tanenbaum_mutex
    if (algo_states[pid] == State::HUNGRY
     && algo_states[left(pid)] != State::EATING
     && algo_states[right(pid)] != State::EATING) {
        algo_states[pid] = State::EATING;        // ASSIGNMENT — not ==
        tanenbaum_cv[pid].notify_one();
    }
}

void eat_tanenbaum(std::size_t pid) {
    {   std::unique_lock lock(tanenbaum_mutex);
        algo_states[pid] = State::HUNGRY;
        tanenbaum_safe_to_eat(pid);
        tanenbaum_cv[pid].wait(lock, [pid]{ return algo_states[pid] == State::EATING; });
    }
    // eat (mutex released, but state holds my place)
    {   std::unique_lock lock(tanenbaum_mutex);
        algo_states[pid] = State::THINKING;
        tanenbaum_safe_to_eat(left(pid));
        tanenbaum_safe_to_eat(right(pid));
    }
}
```

**Mistakes / lessons:**

| # | Mistake | Lesson |
|---|---|---|
| 1 | Single global `std::mutex` | Serialised everything. **Identify the resource being protected** before reaching for the primitive. Per-chopstick mutex is the point. |
| 2 | `std::array<std::mutex, N>` declared INSIDE `eat_*` | Stack-local → each call gets private mutexes → no mutual exclusion. **Sync primitives only work as shared objects across threads.** Promote to namespace scope. |
| 3 | `chopsticks[left].lock()` without matching `unlock()` | `std::mutex` has NO RAII semantics by itself. Plain `{}` braces don't release. Use `lock_guard`/`unique_lock`/`scoped_lock`, or write manual `unlock()`. |
| 4 | Layering `std::scoped_lock` *inside* footman | Two deadlock-prevention mechanisms hide which one is doing the work. Pedagogically muddled. **Write each strategy in its purest form first.** |
| 5 | `algo_states[pid] == State::EATING;` (typo: `==` vs `=`) | Comparison-as-statement; result discarded. **Total deadlock on first meal.** `-Wall -Wextra -Wunused-comparison` would catch it. |

### 4.6 Go — two flavors

```go
const N = 5

type Shared struct {
    chopstickChs [N]chan struct{}                  // ARRAY (zero-init), each chan made in Init()
    // ...
}

func (s *Shared) Init() {
    for i := range s.chopstickChs {
        s.chopstickChs[i] = make(chan struct{}, 1)  // *** cap 1 ***
        s.chopstickChs[i] <- struct{}{}             // prime with one token
    }
}

// Strategy A: odd/even ring (asymmetric ordering)
func (s *Shared) eat(pid int) {
    leftCh := s.getLeftCh(pid)
    rightCh := s.getRightCh(pid)
    if pid % 2 == 0 {
        <-leftCh; <-rightCh                         // even: left first
    } else {
        <-rightCh; <-leftCh                         // odd: right first
    }
    // eat
    leftCh <- struct{}{}; rightCh <- struct{}{}
}

// Strategy B: try-and-back-off (channel equivalent of scoped_lock)
func (s *Shared) eatTryBackoff(pid int) {
    leftCh := s.chopstickChs[pid]
    rightCh := s.chopstickChs[(pid + 1) % N]
acquire:
    for {
        // Phase A: blocking-recv left, non-blocking-try right
        <-leftCh
        select {
        case <-rightCh:
            break acquire                           // labeled break — exits the for, not the select
        default:
        }
        leftCh <- struct{}{}                        // give back left

        // Phase B: blocking-recv right, non-blocking-try left
        <-rightCh
        select {
        case <-leftCh:
            break acquire
        default:
        }
        rightCh <- struct{}{}                       // give back right; loop iterates
    }
    // eat
    leftCh <- struct{}{}; rightCh <- struct{}{}
}
```

**Channel-as-token (Pattern A):** chopstick is a `chan struct{}` with cap 1, primed with one token. Recv = take the chopstick; send = put it back.

**Mistakes / lessons:**

| # | Mistake | Lesson |
|---|---|---|
| 1 | `func (s *Shared) getCh(pid int) chan struct {` (newline before `}`) | Parser sees `chan struct{` and starts reading a STRUCT TYPE DEFINITION. Keep `chan struct{}` on one line. |
| 2 | `} \n else {` | Go's automatic semicolon insertion ends the `if` after `}`. `} else {` MUST be one line. |
| 3 | `s.chopstickChs = make([]chan struct{}, 0, N)` | Type mismatch — array vs slice. Arrays are zero-init by default; only the inner channels need `make`. |
| 4 | `evenIdxCh <- struct{}` | `struct{}` is the **type**; `struct{}{}` is the **value** (empty struct literal `{}` of type `struct{}`). |
| 5 | Try-backoff held both chopsticks forever after first meal | Forgot release at function end. |
| 6 | Bare `break` inside `select` inside `for` | Bare `break` only exits the case. Use **labeled break** to escape the for. |

### 4.7 Rust — five strategies in one binary (tokio)

```rust
async fn eat_asymmetric(shared: &SharedTokio, pid: usize) {
    let (first, second) = if pid == N - 1 {
        ((pid + 1) % N, pid)        // last philosopher reverses
    } else {
        (pid, (pid + 1) % N)
    };
    let _g1 = shared.chopsticks[first].lock().await;     // tokio Mutex<()>
    let _g2 = shared.chopsticks[second].lock().await;
    // eat
    // LIFO drop releases _g2 then _g1
}

async fn eat_try_backoff(shared: &SharedTokio, pid: usize) {
    let left = pid;
    let right = (pid + 1) % N;
    let (_l, _r) = loop {                                // loop is an EXPRESSION
        let l_guard = shared.chopsticks[left].lock().await;
        match shared.chopsticks[right].try_lock() {
            Ok(r_guard) => break (l_guard, r_guard),
            Err(_) => drop(l_guard),
        }
        let r_guard = shared.chopsticks[right].lock().await;
        match shared.chopsticks[left].try_lock() {
            Ok(l_guard) => break (l_guard, r_guard),
            Err(_) => drop(r_guard),
        }
    };
    // eat
}

async fn eat_footman(shared: &SharedTokio, pid: usize) {
    let _diner = shared.num_eaters.acquire().await.unwrap();   // RAII drop refills (pool!)
    let _l = shared.chopsticks[left(pid)].lock().await;
    let _r = shared.chopsticks[right(pid)].lock().await;
    // eat — LIFO drop releases _r, _l, then _diner
}

async fn eat_tanenbaum(shared: &SharedTokio, pid: usize) {
    {
        let mut state = shared.algo_states.lock().await;       // ONE Mutex<[u8; N]>
        state[pid] = HUNGRY;
        shared.tanenbaum_test(&mut state, pid);
    }                                                            // drop guard BEFORE await
    shared.notify[pid].notified().await;                         // tokio::Notify deposit-permit
    // eat
    {
        let mut state = shared.algo_states.lock().await;
        state[pid] = THINKING;
        shared.tanenbaum_test(&mut state, left(pid));
        shared.tanenbaum_test(&mut state, right(pid));
    }
}
```

**Mistakes / lessons:**

| # | Mistake | Lesson |
|---|---|---|
| 1 | `first_chopstick = shared.chopsticks[i].lock().unwrap()` | Three errors: missing `let`; `tokio::Mutex::lock()` returns a Future not Result; `await` not unwrap. |
| 2 | `let mut first = pid; if cond { first = ...; }` | Use `if`-as-expression: `let (first, second) = if cond { (a, b) } else { (c, d) };`. Immutable bindings, clearer intent. |
| 3 | `break;` from retry loop, then use guards outside | Guards die with the loop body. Use `loop`-as-expression: `let (l, r) = loop { ... break (l_guard, r_guard); };`. |
| 4 | `lock().try_lock()` chained | Two siblings, not stacked. `lock().await` returns guard; `try_lock()` returns `Result<Guard, TryLockError>`. |
| 5 | `algo_states[pid].lock().await` | Field is `Mutex<[u8; N]>` (one mutex over an array), not `[Mutex<u8>; N]`. Lock once, get `&mut [u8; N]`, index. |
| 6 | `shared.algo_states.lock().await;` (no `let`) | Guard drops at end of statement → lock released instantly → critical section runs UNLOCKED. **Always `let mut state = ...`.** |
| 7 | `state[pid].store(HUNGRY, Ordering::SeqCst)` | Two state layers conflated. **Oracle** uses `.store()` (atomic). **Algo** uses `=` (plain `u8` under mutex). |
| 8 | `notified().await` while holding the algo lock | Self-deadlock: nobody can acquire the lock to wake you. **Drop the lock BEFORE `.notified().await`.** |
| 9 | "Should I `.forget()` the footman permit?" | **No.** Footman is a **resource pool** — RAII Drop refills the seat. Only the **gate/signal** idiom (barrier) needs `.forget()`. |

#### The pool-vs-gate dichotomy (the most transferable lesson)

| Idiom | Init | `add_permits` | `forget()`? |
|---|---|---|---|
| **Resource pool** (footman, conn pool, rate limit) | `new(capacity)` | never | no — let Drop refund |
| **Gate / one-shot signal** (barrier, broadcast release) | `new(0)` | every round | yes — consume the ticket |

Same `tokio::sync::Semaphore`, opposite semantics — pick wrong and you livelock or leak capacity.

### 4.8 Tanenbaum — semaphore version (worth understanding for "semaphores have memory")

```
takeChopsticks(i):
    wait(mutex)
    state[i] = HUNGRY
    safeToEat(i)              // maybe signals s[i], maybe doesn't
    signal(mutex)
    wait(s[i])                // case A: returns immediately; case B: blocks

putChopsticks(i):
    wait(mutex)
    state[i] = THINKING
    safeToEat(left); safeToEat(right)
    signal(mutex)

safeToEat(i):
    if state[i] == HUNGRY && state[left] != EATING && state[right] != EATING:
        state[i] = EATING
        signal(s[i])
```

`s[i]` per-philosopher, **init 0**. The waiter and the signaler reference the same object. Two cases:
- **A**: `safeToEat(i)` ran the body during my own `takeChopsticks` → permit deposited → my `wait(s[i])` consumes it without blocking.
- **B**: A neighbor was eating → no permit → I block on `wait(s[i])`. Later the neighbor's `putChopsticks` calls `safeToEat(i)`, signals, I wake.

**Key property: semaphores have memory.** A signal arriving before the wait isn't lost — it sits in the counter. Condvars don't have this property.

### 4.9 Exam answer template

1. State the four failure modes (deadlock / livelock / starvation / lost-wakeup).
2. List the five strategies (naive / asymmetric / scoped_lock / footman / Tanenbaum).
3. For each, explain HOW it prevents deadlock — structurally (asymmetric, Tanenbaum) vs runtime (footman) vs library try-and-back-off.
4. Mention the **TSan-vs-runtime gap**: footman has a graph cycle but no actual deadlock (pigeonhole). TSan flags it; a static analyzer can't see "≤N−1".
5. Mention **Tanenbaum's starvation risk** — alternating eaters can starve a hungry middle philosopher. Fix: PRIORITY/WAITING state.
6. **State both Tanenbaum versions** (CV and semaphore) and explain why the semaphore version is "self-healing": deposited signals are remembered.

---

## 5. Barbershop (Sleeping Barber)

### 5.1 Problem statement

1 barber, N waiting chairs. Barber sleeps when no customers. Customers leave (balk) if all chairs full. Customers served one at a time.

### 5.2 English-first algorithm — slide 44 four-semaphore protocol

> **Two handshakes bookending a haircut**, plus a mutex-protected counter.
>
> - **Front handshake (start):** customer signals arrival → barber wakes → barber waves customer in → customer sits.
> - **Cut.**
> - **Back handshake (end):** customer says "done" → barber acknowledges → customer leaves.
>
> Each handshake is a **signal-then-wait pair on each side**. Two semaphores per handshake go in opposite directions.

### 5.3 The structural invariant — the rule that catches all the bugs

Every semaphore has **exactly one releaser and exactly one acquirer:**

| Semaphore | Customer | Barber | Means |
|---|---|---|---|
| `customer_sem` | release | acquire | "a customer arrived" |
| `barber_sem` | acquire | release | "barber is ready" |
| `customer_done` | release | acquire | "customer finished" |
| `barber_done` | acquire | release | "barber acked" |

If your code violates this — both sides releasing, both sides acquiring, or one side touching neither — *that semaphore isn't synchronizing anything*.

### 5.4 Failure modes

| Mode | Triggered by |
|---|---|
| Lost wakeup | Front handshake half-wired (customer doesn't wait for `barber_sem`) — multiple customers sleep "through the haircut" simultaneously. |
| Mutual deadlock | Both sides start with a `send` on unbuffered channels (Go) — neither side's recv is reached. |
| Hang on shutdown | Worker loop only checks `stop_` at top of `while`, not after the wake. Wake from shutdown sentinel → barber proceeds into `customer_done.acquire()` → blocks forever. |
| Lost customer | The unit test only checks `served + balked == TOTAL`. The PROTOCOL can be wrong (simultaneous haircuts!) and still pass — instrument with `active_haircuts` counter. |

### 5.5 C++ implementation

```cpp
class Barbershop {
    std::counting_semaphore<> customer_sem_{0};
    std::counting_semaphore<> barber_sem_{0};
    std::counting_semaphore<> customer_done_sem_{0};
    std::counting_semaphore<> barber_done_sem_{0};
    std::mutex mut_;
    std::atomic<bool> stop_{false};
    int customers_ = 0;                        // counter, protected by mut_
public:
    bool customer(int id) {
        {
            std::unique_lock lk{mut_};
            if (customers_ == CHAIRS) return false;     // BALK
            customers_ += 1;
        }
        customer_sem_.release();                          // "I'm here"  (front handshake)
        barber_sem_.acquire();                            // wait for "your turn"
        std::this_thread::sleep_for(1ms);                 // haircut
        customer_done_sem_.release();                     // "I'm done"  (back handshake)
        barber_done_sem_.acquire();                       // wait for ack, leave
        {
            std::unique_lock lk{mut_};
            customers_ -= 1;
        }
        return true;
    }

    void barber() {
        while (true) {
            customer_sem_.acquire();                      // wait for customer
            if (stop_.load()) break;                      // *** RECHECK after wake ***
            barber_sem_.release();                        // "your turn"
            std::this_thread::sleep_for(1ms);             // cut
            customer_done_sem_.acquire();                 // wait for "I'm done"
            barber_done_sem_.release();                   // "you can leave"
        }
    }

    void stop() {
        stop_.store(true);
        customer_sem_.release();                          // wake the parked barber
    }
};
```

**Mistakes / lessons:**

| # | Mistake | Lesson |
|---|---|---|
| 1 | Customer skipped `barber_sem_.acquire()` between signaling arrival and "haircut" | **Front handshake half-wired.** Multiple customers can "haircut" simultaneously. **Add `active_haircuts` counter** to detect — unit test alone is not enough. |
| 2 | Barber wrote `barber_sem_.acquire()` instead of `release()` | Direction inversion. **Read aloud who signals/waits per semaphore** before writing the line. |
| 3 | "What if I init `barber_sem_{1}` and treat it as a resource lock?" | Doesn't work — barber acquires + releases its own permit, no other thread observes the semaphore. **Pick the signal model**, not the resource model. |
| 4 | Customer wrote `barber_sem_.release()` instead of `acquire()` | Same direction-inversion category. **Both sides release on same semaphore = no synchronization.** |
| 5 | `sleep(1)` instead of `std::this_thread::sleep_for(1ms)` | POSIX `sleep(1)` = 1 second. With 30 customers × 1s = 30s. C++ stdlib answer is `sleep_for(1ms)` with `chrono_literals`. |
| 6 | Barber loop checked `stop_` only at top of `while` | After shutdown wakes barber via `customer_sem_.release()`, the top-check has already passed for THIS iteration → proceeds to `customer_done_sem_.acquire()` → blocks forever. **Recheck *after* the wake, *before* the work.** |
| 7 | Default config (CHAIRS=3, fast cuts) → `Balked=0` every run | **A passing test of one path is not a passing test of the protocol.** Crank contention until balks happen. |

### 5.6 Go implementation

```go
type Barbershop struct {
    chairs       chan int        // BUFFERED cap CHAIRS — queue of waiting customer IDs
    barberReady  chan struct{}   // unbuffered — rendezvous
    customerDone chan struct{}
    barberDone   chan struct{}
    shutdown     chan struct{}
}

func (bs *Barbershop) Customer(id int) bool {
    select {
    case <-bs.shutdown:
        return false
    case bs.chairs <- id:
        <-bs.barberReady                            // wait for "your turn"
        time.Sleep(2 * time.Millisecond)            // haircut
        <-bs.barberDone                             // wait for "you can leave" (back handshake)
        bs.customerDone <- struct{}{}               // "I'm done"
        return true
    default:
        return false                                // BALK
    }
}

func (bs *Barbershop) Barber() {
    for {
        select {
        case <-bs.shutdown:
            return
        case <-bs.chairs:                           // wait for customer
            bs.barberReady <- struct{}{}            // "your turn"
            time.Sleep(2 * time.Millisecond)        // cut
            bs.barberDone <- struct{}{}             // "you can leave"
            <-bs.customerDone                       // wait for "I'm done"
        }
    }
}
```

**Mistakes / lessons:**

| # | Mistake | Lesson |
|---|---|---|
| 1 | `case default:` | `default` is a keyword; `case` doesn't precede it. Bare `default:`. |
| 2 | `for { select { ... } }` where every case `return`s | Loop body never iterates. Drop the `for` for one-shot ops (customer); keep it for long-running workers (barber). |
| 3 | Barber `return` after one customer in cut case | **`return` exits the function, not the case.** Long-running workers' work-arms should NOT return. |
| 4 | Both sides start with a `send` on unbuffered channel | Mutual deadlock — both block on send. **Both sides must mirror each other** (one sends-then-recvs, other recvs-then-sends). Customer-signal pairs with barber-wait. |
| 5 | `case id := <-bs.chairs:` with `id` unused | Go errors on unused locals. Use `case <-bs.chairs:` (discard). |
| 6 | "Surely `for` makes my customer non-blocking" | **No** — `default:` inside `select` is the actual non-blocking knob. `for` only controls iteration. |
| 7 | `make(chan int, Chairs)` thinking it models "total in shop" | Channel buffer = waiting only (not yet picked up). Total in shop = buffer + (1 if cut in progress). To match C++ exactly, use cap `CHAIRS - 1` or add an explicit counter. |

### 5.7 Rust + tokio implementation

```rust
pub struct Barbershop {
    customers: std::sync::Mutex<usize>,        // std::sync — no .await held across
    customer_sem: tokio::sync::Semaphore,
    barber_sem: tokio::sync::Semaphore,
    customer_done: tokio::sync::Semaphore,
    barber_done: tokio::sync::Semaphore,
    stop: AtomicBool,
}

impl Barbershop {
    pub async fn customer(&self, _id: i32) -> bool {
        {
            let mut num_customers = self.customers.lock().unwrap();
            if *num_customers == CHAIRS { return false; }                 // *guard for ops!
            *num_customers += 1;
        }
        self.customer_sem.add_permits(1);                                  // V customer_sem
        self.barber_sem.acquire().await.unwrap().forget();                 // P barber_sem
        tokio::time::sleep(Duration::from_millis(2)).await;
        self.customer_done.add_permits(1);                                 // V customer_done
        self.barber_done.acquire().await.unwrap().forget();                // P barber_done
        {
            let mut num_customers = self.customers.lock().unwrap();
            *num_customers -= 1;
        }
        true
    }

    pub async fn barber(&self) {
        loop {
            self.customer_sem.acquire().await.unwrap().forget();
            if self.stop.load(Ordering::SeqCst) { break; }                 // *** RECHECK ***
            self.barber_sem.add_permits(1);
            tokio::time::sleep(Duration::from_millis(2)).await;
            self.barber_done.add_permits(1);
            self.customer_done.acquire().await.unwrap().forget();
        }
    }

    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.customer_sem.add_permits(1);
    }
}
```

**Mistakes / lessons (most are language quirks, not protocol):**

| # | Mistake | Lesson |
|---|---|---|
| 1 | Customer never V'd `customer_sem` | Same lost-wakeup as C++ #1. Pairing rule catches it. |
| 2 | `customers.lock()` (no `self.`) | Rust has no implicit `this->`. Always `self.field`. |
| 3 | `num_customers == CHAIRS`, `num_customers += 1` on `MutexGuard<usize>` | **Operators don't auto-deref through smart pointers.** Methods do; `==`, `+=`, `<` don't. → `*guard == X`, `*guard += 1`. |
| 4 | `let mut *num_customers = self.customers.lock().unwrap();` | `*` belongs at use site, not binding. → `let mut g = ...; *g += 1;`. |
| 5 | `thread::sleep(...)` inside `async fn` | Blocks the WORKER THREAD, not the task. → `tokio::time::sleep(...).await`. |
| 6 | Same shutdown-recheck bug as C++ #6 | Identical fix: `loop` + `if stop { break; }` after the acquire. |
| 7 | `self.stop_.load()` (C++isms) | Rust uses `pub` for visibility, not name mangling. No trailing `_`. |
| 8 | `.load()` no Ordering | Rust requires explicit `Ordering`; no default. `Ordering::SeqCst` is the right starting point. |

### 5.8 Cross-language comparison

| Concern | C++20 | Go | Rust + tokio |
|---|---|---|---|
| Semaphore primitive | `std::counting_semaphore<>` | `chan struct{}` | `tokio::sync::Semaphore` |
| P (wait) | `.acquire()` | `<-ch` | `.acquire().await.unwrap().forget()` |
| V (signal) | `.release()` | `ch <- struct{}{}` | `.add_permits(1)` |
| Permit lifetime | counter-based, no RAII | channel send/recv pair | RAII — drops return permits unless `.forget()` |
| Counter mutex | `std::mutex` | `sync.Mutex` | `std::sync::Mutex` (no `.await` across) |
| Sleep | `sleep_for(1ms)` | `time.Sleep(...)` | `tokio::time::sleep(...).await` |
| Atomic flag | `std::atomic<bool>` | `atomic.Bool` or `close(chan)` | `AtomicBool` (Ordering required) |

The **structure** of the protocol — two handshakes, four signal pairs, one mutex around the counter, one shutdown signal — is identical across all three. What changes is the syntactic surface.

### 5.9 Exam answer template

> **The two-handshake structure** is the load-bearing insight:
>
> - Front handshake: `customer_sem` (customer→barber) + `barber_sem` (barber→customer).
> - Back handshake: `customer_done_sem` (customer→barber) + `barber_done_sem` (barber→customer).
>
> Each handshake = signal-then-wait pair on each side; semaphores go in opposite directions.
>
> **Per-semaphore, exactly one V'er and one P'er.** If both sides V or both sides P, the semaphore isn't synchronizing.
>
> **Init values match the role:** signal-style → init 0; resource-style → init capacity.
>
> **Shutdown signal needs a recheck *after* the wake.** Worker loop wakes on the same semaphore used for real work; recheck disambiguates "real customer" vs "exit now."
>
> **Test both arms** — served + balked, with active_haircuts assertion.

---

## 6. H2O (Water Factory)

### 6.1 Problem statement

Continuous stream of H and O atoms (each its own thread/goroutine/task). Bond into water molecules: every molecule is **2H + 1O**. Each atom must call its own `bond()` (the three calls forming one molecule must execute together — no atom may bond until 2H+1O have arrived; those exact three bond before any 4th proceeds).

### 6.2 English-first algorithm — three strategies

#### A. Semaphore + Barrier (lecture WaterFactory3)

- Type-cap semaphores limit how many of each kind reach the bonding region.
- A barrier(3) gates the start of bonding so all 3 atoms enter `bond()` together.

```
hydrogen():
    hydrogen_sem.acquire()       // at most 2 H past this line
    barrier.arrive_and_wait()    // wait until 2H + 1O are all here
    bond_h()
    hydrogen_sem.release()       // let the next H in

oxygen():
    oxygen_sem.acquire()         // at most 1 O past this line
    barrier.arrive_and_wait()
    bond_o()
    oxygen_sem.release()
```

Why both primitives:
- Semaphores alone (WaterFactory2) → atoms bond solo without a partner.
- Barrier alone (WaterFactory1) → 3 oxygens can bond into ozone.
- Together → semaphores enforce **type count**, barrier enforces **start signal**.

#### B. Daemon (centralized server)

A long-running goroutine/task owns the protocol. Atoms submit a request and wait for a "go" signal. Atoms are clients; daemon is server.

```
Daemon (forever):
    h1 := <-RequestH; h2 := <-RequestH; o := <-RequestO
    h1 <- "go"; h2 <- "go"; o <- "go"           // signal partners FIRST
    <-h1; <-h2; <-o                              // wait for "done" — completion barrier

Atom (e.g. H):
    commit := make(chan struct{})
    RequestH <- commit                           // precommit: deposit reply chan
    <-commit                                      // commit: "go"
    bond_h()
    commit <- struct{}{}                          // postcommit: "done"
```

The same `commit` channel sees **two turns**: daemon→atom go, then atom→daemon done. Pure synchronization (`struct{}{}` carries no payload).

#### C. Leader election

Same protocol shape as daemon, but the "server" loop body is one iteration of an oxygen's code. A `oxygenMutex` (channel cap 1) ensures only one oxygen runs the orchestration at a time. **No long-running goroutine, no leak.**

### 6.3 Invariants

- Exactly 3 atoms in `bond()` simultaneously per molecule.
- 2 of those are H, 1 is O.
- Atoms bond as themselves (each calls its own `bond_*`).
- After H_total H atoms and O_total O atoms have bonded, all are accounted for.

### 6.4 Failure modes

| Mode | Triggered by |
|---|---|
| Solo bonding | Type-cap semaphores without a barrier (WaterFactory2). |
| Wrong stoichiometry | Barrier without type caps (WaterFactory1). |
| Race-ahead daemon | Missing the postcommit "done" signal — daemon loops to next molecule before the current one finishes bonding. |
| Goroutine leak | Daemon `for { ... }` with no shutdown wiring → leaks forever. |
| Self-deadlock (Go) | `OxySem` channel cap 0 — single goroutine sending and receiving in same function body. |

### 6.5 C++ implementation (WaterFactory3)

```cpp
class WaterFactory {
    std::counting_semaphore<> hydrogen_sem_{2};
    std::counting_semaphore<> oxygen_sem_{1};
    std::barrier<> barrier{3};                            // *** must construct with count ***
public:
    void hydrogen(int id) {
        hydrogen_sem_.acquire();
        barrier.arrive_and_wait();
        bond_h(id);
        hydrogen_sem_.release();
    }
    void oxygen(int id) {
        oxygen_sem_.acquire();
        barrier.arrive_and_wait();
        bond_o(id);
        oxygen_sem_.release();
    }
};
```

**Mistakes / lessons:**

| # | Mistake | Lesson |
|---|---|---|
| 1 | `std::barrier<> barrier;` (no count) | `std::barrier` requires expected count at construction — no default. Same shape as `counting_semaphore<>{N}`. |

> **Anything that *carries a sized state* (barrier, latch, semaphore) needs the count up front.** `std::mutex`, `std::condition_variable`, `std::atomic<T>` are default-constructible — these aren't.

### 6.6 Go implementations

#### A. Daemon strategy

```go
type WaterFactory struct {
    RequestH chan chan struct{}
    RequestO chan chan struct{}
}

func NewWaterFactory() *WaterFactory {
    wf := &WaterFactory{
        RequestH: make(chan chan struct{}),
        RequestO: make(chan chan struct{}),
    }
    go centralManager(wf)                                  // SPAWN the daemon
    return wf
}

func centralManager(wf *WaterFactory) {
    for {                                                   // *** for { } MANDATORY ***
        h1 := <-wf.RequestH
        h2 := <-wf.RequestH
        o  := <-wf.RequestO
        h1 <- struct{}{}; h2 <- struct{}{}; o <- struct{}{} // signal go
        <-h1; <-h2; <-o                                      // wait done — completion barrier
    }
}

func (wf *WaterFactory) hydrogen(id int) {
    commit := make(chan struct{})
    wf.RequestH <- commit                                   // precommit
    <-commit                                                 // commit
    bondH(id)
    commit <- struct{}{}                                     // postcommit (done)
}
```

#### B. Leader strategy (no daemon, no leak)

```go
type WaterFactory struct {
    RequestH chan chan struct{}
    OxySem   chan struct{}                                  // *** cap 1 ***
}

func NewWaterFactory() *WaterFactory {
    return &WaterFactory{
        RequestH: make(chan chan struct{}),
        OxySem:   make(chan struct{}, 1),                   // cap 1, NOT cap 0
    }
}

func (wf *WaterFactory) oxygen(id int) {
    wf.OxySem <- struct{}{}                                 // take leader role (only 1 at a time)
    h1 := <-wf.RequestH
    h2 := <-wf.RequestH
    h1 <- struct{}{}                                         // *** signal partners FIRST ***
    h2 <- struct{}{}
    bondO(id)                                                // then bond concurrently
    <-h1; <-h2                                               // wait done
    <-wf.OxySem                                              // release leader role
}
```

**Mistakes / lessons:**

| # | Mistake | Lesson |
|---|---|---|
| 1 | `:=` inside struct literal | Composite literals use `:` (key-value), not `:=`. |
| 2 | Missing trailing comma in composite literal | Every line of a literal needs a trailing comma. |
| 3 | Forgot to spawn daemon goroutine | Atoms hang on send. **A type with channels is not a service until something `go`-spawns a reader.** |
| 4 | `centralManager` handled ONE molecule | Wrap protocol-per-round in `for { }`. |
| 5 | `OxySem: make(chan struct{})` (cap 0) | Self-deadlock: single goroutine sends and receives. **Same-goroutine take-and-release needs queue semantics → cap 1.** |
| 6 | Leader bonds before signaling partners | All 3 atoms must bond *concurrently*. **Signal partners FIRST, then call your own bond.** |

### 6.7 Rust + tokio implementations

#### A. Semaphore + Barrier

```rust
use tokio::sync::{Semaphore, Barrier};

pub struct WaterFactory {
    oxygen_sem: Semaphore,
    hydrogen_sem: Semaphore,
    barrier: Barrier,
}

impl WaterFactory {
    pub fn new() -> Self {
        Self {
            oxygen_sem: Semaphore::new(1),
            hydrogen_sem: Semaphore::new(2),
            barrier: Barrier::new(3),                       // commas, not semicolons!
        }
    }

    pub async fn hydrogen(&self, id: usize) {
        self.hydrogen_sem.acquire().await.unwrap().forget(); // hold across barrier+bond
        self.barrier.wait().await;
        bond_h(id).await;
        self.hydrogen_sem.add_permits(1);
    }
}
```

#### B. Daemon strategy

```rust
struct Request {
    go: oneshot::Sender<()>,                                  // daemon→atom
    done: oneshot::Receiver<()>,                              // atom→daemon
}

pub struct WaterFactory {
    h_tx: mpsc::Sender<Request>,
    o_tx: mpsc::Sender<Request>,
}

async fn manager_loop(
    mut h_rx: mpsc::Receiver<Request>,
    mut o_rx: mpsc::Receiver<Request>,
) {
    loop {
        let h1 = h_rx.recv().await.unwrap();
        let h2 = h_rx.recv().await.unwrap();
        let o  = o_rx.recv().await.unwrap();
        h1.go.send(()).unwrap(); h2.go.send(()).unwrap(); o.go.send(()).unwrap();
        h1.done.await.unwrap(); h2.done.await.unwrap(); o.done.await.unwrap();
    }
}

impl WaterFactory {
    pub fn new() -> Self {
        let (h_tx, h_rx) = mpsc::channel(N_HYDROGEN);
        let (o_tx, o_rx) = mpsc::channel(N_OXYGEN);
        tokio::spawn(manager_loop(h_rx, o_rx));
        Self { h_tx, o_tx }
    }

    pub async fn hydrogen(&self, id: usize) {
        let (go_tx, go_rx) = oneshot::channel();
        let (done_tx, done_rx) = oneshot::channel();
        self.h_tx.send(Request { go: go_tx, done: done_rx }).await.unwrap();
        go_rx.await.unwrap();
        bond_h(id).await;
        done_tx.send(()).unwrap();
    }
}
```

**Why TWO oneshots per atom (vs Go's single channel):**

Go reuses one bidirectional `chan struct{}` for both go and done because Go channels are *reusable*. Rust's `oneshot` is **single-shot** — once the value goes through, the channel is dead. So you need two oneshots per atom: one for daemon→atom go, one for atom→daemon done.

**Mistakes / lessons:**

| # | Mistake | Lesson |
|---|---|---|
| 1 | `use std::sync::Barrier;` inside async | std barrier blocks the worker thread. Use `tokio::sync::Barrier`. |
| 2 | Stray `;` inside struct literal | Struct literals take `field: value` pairs separated by `,`. Statements (`let`, `tokio::spawn`) come *before* the literal. |
| 3 | Missing `self.` on field access | Rust has no implicit `this`. |
| 4 | Missing `.await` on `barrier.wait()` | Future is created and immediately dropped; barrier never engages. |
| 5 | `&mut h_rx: mpsc::Receiver<Request>` parameter | That's a pattern-match. Use `mut h_rx: T`. |
| 6 | `manager_loop` declared as method `&self` | Long-running task should *own* state, not borrow. Free `async fn`, move state in. |
| 7 | `h1.done.send(()).unwrap()` | `done` is a `Receiver`. Receivers don't have `.send` — they have `.await`. |

### 6.8 Cross-language comparison

| Aspect | C++ | Go (daemon) | Go (leader) | Rust + tokio (sem+barrier) | Rust + tokio (daemon) |
|---|---|---|---|---|---|
| Long-running task | n/a | yes (leak risk) | **no** | n/a | yes |
| Per-atom code | very short | minimal | minimal | very short | minimal |
| Coordination layer | barrier + sem | daemon goroutine | leader-elected oxygen | barrier + sem | daemon task + 2 oneshots/atom |
| Shutdown discipline | n/a | needs explicit | n/a | n/a | needs explicit |

### 6.9 Exam answer template

> Three idiomatic strategies, each with a different center of gravity:
>
> 1. **Semaphore + Barrier (textbook).** Type-cap semaphores + barrier(3). Compact, stateless, no long-running task. The semaphore must be **held across the barrier and the bond** — release before bond and a 4th atom rushes in.
> 2. **Daemon.** Long-running task orchestrates the protocol. Each atom submits a request (with reply channel) and waits for "go," then bonds, then signals "done." Postcommit "done" is a **completion barrier** — without it the daemon races into the next molecule.
> 3. **Leader.** Like daemon, but the "leader" is one of the atoms (typically the oxygen, since there's only 1 per molecule). Same protocol, no goroutine leak. Channel-as-mutex must be **cap 1**, not cap 0 (cap 0 self-deadlocks in single-goroutine code).
>
> **Why both type-cap AND barrier**: semaphores alone allow solo bonding; barrier alone allows 3-O ozone. Together they pin both type and timing.

---

## 7. FIFO Semaphore

### 7.1 Problem statement

A counting semaphore with the additional guarantee that waiters are unblocked in **FIFO order** of their `acquire()` calls. The lecture covers multiple implementation strategies; we did three.

### 7.2 English-first algorithms

#### Strategy 1: Ticket queue + condvar (FIFOSemaphore2 / Task 1)

```
state: next_ticket (atomic), now_serving (atomic), shared cv

acquire():
    my_ticket = next_ticket.fetch_add(1)         // get a ticket
    cv.wait while my_ticket > now_serving          // park until counter reaches me

release():
    LOCK
    now_serving++                                  // mutation under the lock
    UNLOCK
    cv.notify_all()                                // wake everybody to recheck
```

#### Strategy 2: Queue of per-waiter binary semaphores (FIFOSemaphore5)

```
state: count (int), waiters (queue of binary_semaphore)

acquire():
    LOCK
    if count > 0: --count; UNLOCK; return         // permit available, no queue
    waiter = new Waiter
    waiters.push(waiter)
    UNLOCK
    waiter.sem.acquire()                           // park OUTSIDE the lock

release():
    LOCK
    if waiters.empty(): ++count; UNLOCK; return   // no waiter, bank the permit
    waiter = waiters.front(); waiters.pop()
    UNLOCK
    waiter.sem.release()                           // direct hand-off
```

#### Strategy 3: Daemon (Go-style) / oneshot queue (Rust)

A goroutine/task owns `count` and `waiters`. Two channels: `acquireCh chan chan struct{}` and `releaseCh chan struct{}`. Daemon's `for { select { ... } }` loop reacts to whichever fires.

### 7.3 The two-branch invariant for `release()`

> **A release does exactly one thing per call: wake a queued waiter OR bump count, never both.**
>
> If `waiters.empty()`: increment count.
> Else: pop front, hand permit directly to that waiter (their semaphore/oneshot/channel).

This is the rule that catches the ticket queue's #11 bug and the queue version's #6 bug.

### 7.4 Lost-wakeup hazard (the conceptual centerpiece)

```cpp
// BUGGY release()
void release() {
    now_serving.fetch_add(1);     // mutated OUTSIDE the lock
    cv.notify_all();
}
```

Race window:

```
T (acquirer)              R (releaser, no lock)
LOCK
pred() → false
                         now_serving++
                         cv.notify_all()        ← wakes nobody (T not parked yet)
[wait:
   UNLOCK
   park on cv]                                  ← T parks AFTER notify, misses it
... hangs forever
```

**`cv.notify_all` has NO memory.** Notifying into an empty cv vaporizes the signal. Atomicity of the value isn't enough — `cv.wait`'s "release lock atomically with parking" guarantee is only atomic w.r.t. **other holders of the same mutex**. If the mutator doesn't take the lock, it can race past the unlock-and-park.

**Fix:** lock briefly during the mutation. Notify can be outside the lock; what matters is that the *mutation* serializes with the predicate evaluation:

```cpp
void release() {
    { std::scoped_lock lk{mut_}; ++now_serving; }
    cv.notify_all();
}
```

### 7.5 Strategy 2's structural advantage: lost-wakeup IMMUNE

| | cv | binary_semaphore |
|---|---|---|
| Memory of past notifications | **No** — notify into empty cv vaporizes | **Yes** — release-before-acquire just sets the flag |
| Wake-up window | release must hold the same lock as the waiter | release can fire at any time, even before the waiter parks |
| Wait set | shared (one cv, many waiters) | private (one semaphore per waiter) |

In Strategy 2, by the time the releaser pops a waiter and calls `sem.release()`, that waiter is already in the queue. Even if `sem.release()` fires before the waiter reaches `sem.acquire()`, the binary_semaphore stores the permit. **Structurally lost-wakeup-immune.**

### 7.6 C++ — both strategies

#### Strategy 1: ticket queue + condvar

```cpp
class FifoSemaphore {
    std::mutex mut_;
    std::condition_variable cv;
    std::atomic<std::ptrdiff_t> next_ticket{1};
    std::atomic<std::ptrdiff_t> now_serving;
public:
    explicit FifoSemaphore(std::ptrdiff_t initial_count)
      : now_serving{initial_count} {}

    void acquire() {
        std::unique_lock lk{mut_};
        auto my_ticket = next_ticket.fetch_add(1);
        cv.wait(lk, [my_ticket, this]{ return my_ticket <= now_serving; });
    }

    void release() {
        { std::scoped_lock lk{mut_}; ++now_serving; }      // *** mutation under lock ***
        cv.notify_all();
    }
};
```

**Mistakes / lessons (Strategy 1):**

| # | Mistake | Lesson |
|---|---|---|
| 1 | Lambda missing `return` | `cv.wait`'s predicate must return bool. Bare expression statement returns `void`. |
| 2 | Predicate `>=` (false at equality), then `<` (false at equality) | Right form is `<=`. **`now_serving` is "highest ticket cleared so far"** — proceed when counter has reached your ticket. |
| 3 | Forgot to init `now_serving` from `initial_count` | First `initial_count` calls must walk through unblocked. **Init values, predicate direction, ticket numbering are three sides of one design choice.** |
| 4 | Lost wakeup in `release()` | Mutate under the lock, even if the value is atomic. Atomicity ≠ memory model wrt cv parking. |

#### Strategy 2: queue of per-waiter semaphores

```cpp
struct Waiter { std::binary_semaphore sem{0}; };

class FifoSemaphore {
    std::mutex mut_;
    int count_;
    std::queue<std::shared_ptr<Waiter>> waiters_;
public:
    explicit FifoSemaphore(int initial_count) : count_(initial_count) {}

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
        waiter->sem.release();
    }
};
```

**Mistakes / lessons (Strategy 2):**

| # | Mistake | Lesson |
|---|---|---|
| 5 | `elapsed.count_()` from a clobbering rename | Rename `count` → `count_` ate `chrono::duration::count()`. **Use scoped/identifier-aware refactoring.** |
| 6 | `release()` always pops, even on empty queue | `std::queue::front()` on empty is **UB**. Check `empty()` first. |
| 7 | `release()` increments `count_` AND wakes a waiter | Two-branch invariant: emit exactly ONE permit per call. |

### 7.7 Go — daemon strategy

```go
type FifoSemaphore struct {
    acquireCh chan chan struct{}                          // chan-of-chan: pass reply line
    releaseCh chan struct{}                                // bare signal
}

func NewFifoSemaphore(initial int) *FifoSemaphore {
    s := &FifoSemaphore{
        acquireCh: make(chan chan struct{}),               // unbuffered fine
        releaseCh: make(chan struct{}),
    }
    go func() {
        count := initial
        waiters := list.New()
        for {
            select {
            case <-s.releaseCh:
                if waiters.Len() > 0 {
                    e := waiters.Front()
                    waiters.Remove(e)
                    e.Value.(chan struct{}) <- struct{}{}  // direct hand-off
                } else {
                    count++                                  // bank
                }
            case ch := <-s.acquireCh:
                if count > 0 {
                    count--
                    ch <- struct{}{}                          // immediate wake
                } else {
                    waiters.PushBack(ch)                      // queue
                }
            }
        }
    }()
    return s
}

func (s *FifoSemaphore) Acquire() {
    ch := make(chan struct{})
    s.acquireCh <- ch
    <-ch
}

func (s *FifoSemaphore) Release() { s.releaseCh <- struct{}{} }
```

**Why no mutex needed:** `count` and `waiters` are **local variables inside the daemon goroutine**. Single-goroutine ownership of mutable state ⇒ no synchronization primitives required. The channels do the synchronization.

**Mistakes / lessons:**

| # | Mistake | Lesson |
|---|---|---|
| 1 | `select <-channel :` not valid | `select` is **always a block** with `case` arms. |
| 2 | `} else {` on two lines | ASI ends the `if`. Same line. |
| 3 | `go func(){...}` without `()` | `go` requires a *call*. Always `go func(){...}()`. |
| 4 | `releaseCh chan chan struct{}` (asymmetry confusion) | Acquirers need reply channels (chan-of-chan); releasers don't (bare `chan struct{}`). |
| 5 | Forgot `import "container/list"` | Cascade of `<X> undefined` errors. Suspect missing import. |
| 6 | Two sequential `select`s instead of one with two cases | Forces alternation: release-then-acquire-then-release. **One `select` with multiple cases for "react to whichever fires first."** |

### 7.8 Rust — two strategies

#### Strategy 2: ticket + per-ticket Notify (no mutex)

```rust
const SLOTS: usize = N_THREADS + INITIAL_COUNT + 1;

pub struct FifoSemaphore {
    now_serving: AtomicUsize,
    next_ticket: AtomicUsize,
    notify_list: [Notify; SLOTS],
}

impl FifoSemaphore {
    pub fn new(initial_count: usize) -> Self {
        let s = Self {
            now_serving: AtomicUsize::new(initial_count),
            next_ticket: AtomicUsize::new(0),
            notify_list: std::array::from_fn(|_| Notify::new()),
        };
        for i in 0..initial_count { s.notify_list[i].notify_one(); }   // pre-deposit
        s
    }

    pub async fn acquire(&self) {
        let t = self.next_ticket.fetch_add(1, Ordering::SeqCst);
        self.notify_list[t].notified().await;
    }

    pub fn release(&self) {
        let t = self.now_serving.fetch_add(1, Ordering::SeqCst);
        self.notify_list[t].notify_one();
    }
}
```

**Why per-ticket Notify avoids the lost-wakeup AND the FIFO-fairness traps:**

A single shared `Notify` would have *both* problems: (a) load-then-await window where another waiter races; (b) `notify_one` could wake the wrong waiter (whose ticket isn't ready). Per-ticket Notify:
- Wakeup is unambiguous — only the release that bumps past my ticket notifies my slot.
- Lost-wakeup is handled by Notify's **deposited-permit property**: if `notify_one` fires before `notified().await` registers, the permit sits on that slot.
- `notified().await` collapses to one line.

#### Strategy 3: oneshot queue

```rust
struct State {
    count: usize,
    waiters: VecDeque<oneshot::Sender<()>>,
}

pub struct FifoSemaphore { state: Mutex<State> }

impl FifoSemaphore {
    pub async fn acquire(&self) {
        let rx = {
            let mut state = self.state.lock().unwrap();
            if state.count > 0 { state.count -= 1; return; }
            let (tx, rx) = oneshot::channel();
            state.waiters.push_back(tx);
            rx
        };  // *** state guard MUST be lexically dead before .await — Send analysis ***
        rx.await.unwrap();
    }

    pub fn release(&self) {
        let mut state = self.state.lock().unwrap();
        if let Some(tx) = state.waiters.pop_front() {
            drop(state);
            let _ = tx.send(());
        } else {
            state.count += 1;
        }
    }
}
```

**Mistakes / lessons:**

| # | Mistake | Lesson |
|---|---|---|
| 1 | Holding `Mutex` across `.await` | `MutexGuard` is `!Send` → future is `!Send` → `tokio::spawn` rejects. **Use block-scope** to make the guard lexically dead before the await. **`drop(guard)` does NOT help.** |
| 2 | `acquire()` doesn't early-return on fast path | Permits leak; silent deadlock. |
| 3 | `let mut count = state.count; count += 1;` | Copy trap — increments local. Mutate `state.count` directly. |
| 4 | `tx.send(()).unwrap()` panics on cancelled receiver | Use `let _ = tx.send(());` for release semantics. |

### 7.9 Cross-language comparison

| Aspect | C++ ticket + cv | C++ queue + per-waiter | Go daemon | Rust per-ticket Notify | Rust oneshot queue |
|---|---|---|---|---|---|
| Coordination state | shared cv + atomics | mutex + queue + per-waiter sem | local in daemon goroutine | atomics + per-ticket Notify | mutex + VecDeque + counter |
| Lost-wakeup hazard | **YES** (mitigate with mutate-under-lock) | **NO** (binary_sem has memory) | n/a (channel rendezvous) | **NO** (Notify deposit-permit) | n/a (oneshot deposit) |
| Mutex needed? | yes | yes | no (single-goroutine ownership) | no | yes |
| Capacity | unbounded | unbounded | unbounded | fixed slot array | unbounded |

### 7.10 Exam answer template

> A FIFO semaphore is `count + queue-of-waiters`. Strategies vary in how the queue is materialized:
>
> 1. **Implicit queue (ticket).** `next_ticket`, `now_serving`, predicate `my_ticket <= now_serving`, single shared cv. Lost-wakeup hazard if `release()` mutates outside the lock.
> 2. **Explicit queue (per-waiter binary semaphore).** Each waiter has its own `binary_semaphore`; the queue stores them. Direct hand-off on release. **Structurally lost-wakeup-immune** because binary_semaphore has memory; cv does not.
> 3. **Daemon.** A goroutine/task owns count and queue; channels are the API. Single-goroutine ownership ⇒ no mutex needed.
>
> **Two-branch invariant for `release()`:** wake a queued waiter OR bump count, never both.
>
> **The lost-wakeup deep cut:** `cv.notify` has no memory. Mutating predicate state outside the lock can race past the waiter's atomic unlock-and-park. **Always mutate under the same mutex the waiter waits with**, even if the value itself is atomic.

---

## 8. Search-Insert-Delete

### 8.1 Problem statement

Three roles share a list (Allen Downey, *Little Book of Semaphores* 6.1.2):

- **Searchers** examine the list. Many can run simultaneously.
- **Inserters** append to the tail. **At most one** at a time, but inserter and any number of searchers can co-exist.
- **Deleters** remove. **Exclusive** — no searchers, no inserters concurrently.

This is the **three-role generalization of Readers-Writers**. Searcher↔reader, deleter↔writer, and *inserter* is a new "compatible with searchers but mutually exclusive with itself and with deleters" role.

### 8.2 Concurrency rules table

| Role pair | Compatible? |
|---|---|
| Searcher / Searcher | ✓ unbounded |
| Searcher / Inserter | ✓ |
| Searcher / Deleter | ✗ |
| Inserter / Inserter | ✗ |
| Inserter / Deleter | ✗ |
| Deleter / Deleter | ✗ |

> **Why is searcher + inserter safe?** The inserter only mutates the **tail** (one pointer write); searchers walk the interior. They touch disjoint memory. A searcher may *miss* a newly-appended item — but that's a legal linearization (search-before-insert), not a correctness violation. See universal lesson **K** ("concurrency rules ARE consistency contracts").

### 8.3 Invariants

```
0 ≤ searcher_count
0 ≤ inserter_count ≤ 1
deleter_count > 0  ⇒  searcher_count == 0 && inserter_count == 0
```

Track all three with `std::atomic<int>` in an oracle struct, asserted at every `enter_*` / `exit_*` boundary. Same shape as `ActiveCounters` in §2.

### 8.4 Failure modes

| Mode | Triggered by |
|---|---|
| Searcher enters during delete | Searchers don't acquire `no_searcher` first-in/last-out → deleter holds `no_searcher` but searcher walks in anyway |
| Two inserters concurrent | Forgot the inserter mutex / `no_inserter` semaphore |
| Deleter starvation | Steady searcher load keeps `no_searcher` always held → no turnstile |
| Deleter–deleter deadlock | Two deleters acquire `no_searcher` and `no_inserter` in opposite orders → cycle |
| TSan-flagged data race in container | `std::list::push_back` / `std::vector::push_back` write without release barrier; concurrent searcher iteration is a race per C++ memory model even if sync-correct |

### 8.5 C++ — two implementations

#### A. Lightswitch (deleter-can-starve)

```cpp
class SidListLightswitch {
    std::mutex mtx_;                                  // guards searcher_count_
    int searcher_count_ = 0;
    std::counting_semaphore<1> no_searcher_{1};       // held while ≥1 searcher in
    std::counting_semaphore<1> no_inserter_{1};       // held while inserter in
    // ... data storage with atomic-size publication; see §8.7 ...
public:
    void search(int x) {
        mtx_.lock();
        ++searcher_count_;
        if (searcher_count_ == 1) no_searcher_.acquire();   // first searcher locks out deleters
        mtx_.unlock();
        // -- actual search (no locks held) --
        mtx_.lock();
        --searcher_count_;
        if (searcher_count_ == 0) no_searcher_.release();   // last searcher yields
        mtx_.unlock();
    }
    void insert(int x) {
        no_inserter_.acquire();                              // exclude other inserters AND deleters
        // -- actual insert --
        no_inserter_.release();
    }
    void delete_one() {
        no_searcher_.acquire();                              // lock order: no_searcher → no_inserter
        no_inserter_.acquire();
        // -- actual delete --
        no_inserter_.release();                              // LIFO release
        no_searcher_.release();
    }
};
```

Key shape decisions:
- **Searchers** use the lightswitch (counter + first/last). Counter needed because >1 searcher can be in.
- **Inserters** use a single binary semaphore — no counter, because only 1 inserter is ever in. The semaphore IS the count. (Universal lesson **I**.)
- **Deleter** acquires both rooms in a fixed order. Releases LIFO. All deleters MUST agree on the order or you get circular-wait deadlock.
- `no_searcher` and `no_inserter` are **binary semaphores, not mutexes**: they're acquired by one thread (first searcher / inserter) and released by another (last searcher / deleter on its own behalf). Cross-thread release → semaphore. (Universal lesson **J**.)

#### B. No-starve turnstile

Same as §2.5 turnstile shape — wrap every entrant in a turnstile gate-pass; deleter holds it across its full acquisition path:

```cpp
void search(int x) {
    turnstile_.acquire(); turnstile_.release();           // gate-pass
    // ... lightswitch as above ...
}
void insert(int x) {
    turnstile_.acquire(); turnstile_.release();           // gate-pass
    no_inserter_.acquire();
    // ... actual insert ...
    no_inserter_.release();
}
void delete_one() {
    turnstile_.acquire();                                 // HELD across both acquires
    no_searcher_.acquire();
    no_inserter_.acquire();
    // ... actual delete ...
    no_inserter_.release();
    no_searcher_.release();
    turnstile_.release();
}
```

Once a deleter is queued on `no_searcher`, the turnstile is held → new searchers and new inserters can't slip past while existing searchers drain. Same FIFO-wakeup caveat as §2.8.

### 8.6 Mistake catalogue

| # | Mistake | Why it bites |
|---|---|---|
| 1 | Initialize `no_searcher`/`no_inserter` to **0** instead of 1 | Inverted semantics; first searcher blocks forever at startup. The "1" means "the room is *available*", not "there's 1 thread inside." |
| 2 | Hold `mtx_` across the actual search | Serializes searchers, kills the parallelism the lightswitch was designed to enable. `mtx_` should only bracket counter mutations. |
| 3 | Add an `is_deleter_active` boolean | Redundant. The state "both `no_searcher` and `no_inserter` are at 0" already encodes deleter-active. Two sources of truth → drift bug. (Universal lesson **L**.) |
| 4 | Add a counter for inserters | Unnecessary — at most 1 inserter is ever in, the semaphore is its own count. The counter is only needed when the role is unbounded. |
| 5 | Deleter acquires `no_inserter` before `no_searcher` | Two deleters can deadlock if they pick opposite orders. Document a global lock order. |
| 6 | Forget to release one of the semaphores in `delete_one` | One delete and the system wedges; subsequent searchers/inserters block forever. |
| 7 | Use `std::list<T>` as the storage | TSan flags `push_back` ↔ iteration as a race. Even though Downey's *abstraction* says searcher+inserter is safe, `std::list` doesn't honor it (no release/acquire). Use a pre-allocated `std::vector<T>` + `atomic<size_t>` for publication. |
| 8 | Drop the turnstile and expect fairness | Steady searcher load → `no_searcher` never released → deleter starves. Turnstile is the standard fix from §2.5. |

### 8.7 Storage detail — the `std::list` footgun

The synchronization rules (searcher + inserter compatible) require a data structure where the inserter publishes new state to concurrent searchers via a release/acquire boundary. `std::list` doesn't — its `push_back` writes a `next` pointer with no synchronization, so even a sync-correct program triggers a TSan data race.

The minimal honest fix:

```cpp
constexpr size_t MAX_SLOTS = 1 << 14;
class SidList {
    std::vector<int> data_;                 // pre-sized to MAX_SLOTS (no realloc ever)
    std::atomic<size_t> size_{0};           // logical valid count
public:
    SidList() : data_(MAX_SLOTS) {}

    // inside insert (exclusive among inserters):
    size_t i = size_.load(std::memory_order_relaxed);
    data_[i] = x;
    size_.store(i + 1, std::memory_order_release);          // publish

    // inside search:
    size_t n = size_.load(std::memory_order_acquire);       // see all writes ≤ n
    for (size_t i = 0; i < n; ++i) sink += data_[i];
};
```

The `release`/`acquire` on `size_` is the missing happens-before that the abstraction quietly assumed. Same shape as Pattern **C12** (atomic-size append).

### 8.8 Exam answer template

1. State the three concurrency rules explicitly (§8.2 table). The "searcher + inserter compatible" rule is the load-bearing one — it's why this is harder than RW.
2. Three rooms / two semaphores: `no_searcher` (lightswitch on, with counter+mtx), `no_inserter` (binary, no counter). Optionally add `turnstile` for fairness.
3. Lock order on the deleter (`no_searcher` → `no_inserter`, LIFO release). Mention deadlock-by-circular-wait if any actor acquires them differently.
4. **Searcher uses a counter; inserter doesn't.** Justify with universal lesson **I** — counter exists only when the role allows >1 concurrent participants.
5. **Both rooms are semaphores not mutexes** — first-acquirer ≠ last-releaser. Cross-thread release rules out `std::mutex`.
6. Without a turnstile, deleters starve under steady searcher load — same as RW §2.
7. (Bonus) Note the consistency contract: searcher + inserter co-existing means `search` may miss in-flight inserts, which is a legal linearization. (Universal lesson **K**.)

---

## 9. Bridge Crossing

### 9.1 Problem statement

A narrow bridge holds up to **K = 4** cars at once. Cars travel north or south. **Same-direction cars may share the bridge; opposite-direction cars must NOT be on it simultaneously.** This is the **two-writer-type generalization of Readers-Writers**: instead of one role being concurrent (readers) and one being exclusive (writers), both roles allow same-role concurrency, and the constraint is "the bridge has a *current direction* that excludes the other."

Equivalent classics: Tanenbaum's Toilet Problem (K = ∞), Tanenbaum's Pipe Problem.

### 9.2 Concurrency rules

| State on bridge | New northbound car can enter? | New southbound car can enter? |
|---|---|---|
| Empty | ✓ | ✓ |
| Northbound, count < K, no south waiting | ✓ (joins) | ✗ |
| Northbound, count < K, south waiting | ✗ (anti-barge) | ✗ (must wait its turn) |
| Northbound, count == K | ✗ | ✗ |
| Symmetric for southbound | | |

### 9.3 Invariants

```
0 ≤ on ≤ K
on > 0  ⇒  all cars on bridge share one direction
eventually every car crosses (no starvation)
```

### 9.4 Failure modes

| Mode | Triggered by |
|---|---|
| Direction-mix violation | Predicate forgot the opposite-count check (`countS == 0` for north entry) |
| Lightswitch-style starvation | New same-direction cars allowed to barge in even while opposite is queued |
| Deadlock at handoff | Bridge drains with both directions queued; both predicates require "opposite not waiting" → mutual block. Fix is direction handoff at exit (§9.5) |
| Lost wakeup on drain | Forgot `cv.Broadcast()` when `on` reaches 0; queued opposite never wakes |
| Capacity violation under jitter | Predicate is `count <= K` instead of `count < K`; one extra car slips in at boundary |

### 9.5 Go implementation — `sync.Mutex` + `sync.Cond` with explicit direction handoff

The predicate has THREE clauses:
1. `on < K` — capacity gate.
2. `dir == myDir || dir == DirFree` — direction match.
3. `on == 0 || oppWait == 0` — anti-barge gate (only refuse to barge if bridge is non-empty AND opposite is queued).

The third clause prevents starvation under steady same-direction load. Without it, you have lightswitch-style starvation. Without `on == 0` carve-out, you have a deadlock at drain when both directions are queued.

```go
package main

import "sync"

const (
    K        = 4
    DirFree  = 0
    DirNorth = 1
    DirSouth = -1
)

type Bridge struct {
    mu    sync.Mutex
    cv    *sync.Cond
    on    int   // cars currently on bridge
    dir   int   // current direction (free/north/south)
    waitN int   // cars parked at north entry
    waitS int   // cars parked at south entry
}

func NewBridge() *Bridge {
    b := &Bridge{}
    b.cv = sync.NewCond(&b.mu)
    return b
}

func (b *Bridge) Enter(north bool) {
    b.mu.Lock()
    defer b.mu.Unlock()

    var myDir int
    var myWait, oppWait *int
    if north {
        myDir = DirNorth
        myWait, oppWait = &b.waitN, &b.waitS
    } else {
        myDir = DirSouth
        myWait, oppWait = &b.waitS, &b.waitN
    }

    *myWait++
    for !(b.on < K &&
        (b.dir == myDir || b.dir == DirFree) &&
        (b.on == 0 || *oppWait == 0)) {
        b.cv.Wait()
    }
    *myWait--

    if b.dir == DirFree {
        b.dir = myDir
    }
    b.on++
}

func (b *Bridge) Exit() {
    b.mu.Lock()
    defer b.mu.Unlock()

    b.on--
    if b.on == 0 {
        // Direction handoff. Prefer the OPPOSITE direction if queued
        // (this is what breaks starvation); else fall through to whoever
        // is waiting; else mark the bridge free.
        switch {
        case b.dir == DirNorth && b.waitS > 0:
            b.dir = DirSouth
        case b.dir == DirSouth && b.waitN > 0:
            b.dir = DirNorth
        case b.waitN > 0:
            b.dir = DirNorth
        case b.waitS > 0:
            b.dir = DirSouth
        default:
            b.dir = DirFree
        }
        b.cv.Broadcast()
    }
}
```

Caller shape (the harness):

```go
for i := 0; i < N_CARS; i++ {
    go func(north bool) {
        bridge.Enter(north)
        // cross — random duration 1..10ms, asserted invariants here
        bridge.Exit()
    }(rng.Float64() < 0.6)
}
```

### 9.6 Why each clause is load-bearing

| Drop... | What breaks |
|---|---|
| `on < K` from predicate | Bridge capacity violated under load |
| `dir == myDir \|\| dir == DirFree` | Opposite-direction cars enter while bridge is wrong-way → invariant 2 fires |
| `on == 0 \|\| oppWait == 0` | Steady same-direction stream barges in past queued opposite → starvation |
| The `on == 0` carve-out specifically | Bridge drains, both sides queued, BOTH predicates fail (each says "opposite is waiting"), deadlock |
| Direction handoff in `Exit` | After drain, dir stays at old direction; opposite waiters never proceed even after broadcast — predicate fails on `dir == myDir` |
| `cv.Broadcast()` | Lost wakeup. Only one waiter wakes; if their predicate happens to fail, no one is left to wake the others |

### 9.7 Mistake catalogue

| # | Mistake | Why it bites |
|---|---|---|
| 1 | Forget `*myWait++` before the wait loop | Other side can't see you're queued; opposite-anti-barge check thinks the gate is clear |
| 2 | Decrement `*myWait` outside the lock | Race; another thread can read stale value |
| 3 | Use `cv.Signal()` instead of `Broadcast()` on drain | Wakes only one waiter; others stay parked. Many waiters wake, but only some have valid predicates |
| 4 | Predicate is `dir != opposite` instead of `dir == myDir \|\| dir == DirFree` | Subtle: works in steady state but breaks on the `DirFree` initial / post-drain state |
| 5 | Skip the `dir` enum, just use counts | "Direction handoff" can't be implemented cleanly; you get the both-sides-queued deadlock |
| 6 | Reset `dir = DirFree` unconditionally on drain | Defeats handoff; both predicates allow entry, race resolves randomly, starvation possible |
| 7 | Stretch — `Enter` takes the lock, sleeps `cross_duration`, then `Exit` releases | The lock is held across the cross — only one car on the bridge at a time. Lock must be released between Enter and Exit |

### 9.8 The "extra credit" strict-FIFO variant (Hint 3)

Strict FIFO admits cars in arrival order regardless of direction — a single south-going car held in queue stalls a stream of north-goings. Trade-off: eliminates starvation entirely but kills same-direction concurrency.

Implementation sketch: each car gets a ticket on arrival; a single "current ticket" advances; cars busy-wait or condvar-wait until their ticket is current. Bridge holds at most K consecutive same-direction tickets at the head of the queue.

You almost never want this in practice — alternation fairness (§9.5) is the right default.

### 9.9 Exam answer template

1. State the rules: `on ≤ K`; `on > 0 ⇒ single direction`. Note this is RW with two writer-types.
2. State the FOUR pieces of state: `on`, `dir`, `waitN`, `waitS`. Explain why all four are needed (anti-barge needs the waiting counts; handoff needs `dir`).
3. Three-clause predicate: capacity, direction-match, anti-barge (with `on==0` carve-out). Justify each clause with the failure mode it prevents (§9.6).
4. **Direction handoff in Exit**: opposite-first to prevent starvation. Without it, you get a deadlock when both sides are queued at drain.
5. **Broadcast on drain, not signal** — many waiters may have valid predicates after the handoff.
6. Mention the strict-FIFO variant only if asked; default is alternation fairness.

---

## Universal cross-cutting lessons

(These appear repeatedly and deserve a section all to themselves — internalize them before any exam.)

### A. The instrumentation oracle is independent of the algorithm

Every template uses `counters_` / `Counters` / `ActiveCounters` as a **runtime invariant witness** — separate from solution state. Don't conflate them.

> If you're writing an algorithm and an invariant checker for the same piece of state, keep them as **two separate variables** even though they conceptually track the same thing. The redundancy is what lets the checker catch bugs in the state machine.

### B. Per-op atomicity ≠ multi-op atomicity (the #1 transferable bug)

```cpp
--rc_;                                  // atomic
if (rc_ == 0) sem.release();           // separate atomic load
```

Two threads can both observe 0 and both `release()`. Mutex must wrap the **whole** sequence; or use `compare_exchange_weak`.

> **`std::atomic` (and `AtomicI32`, `atomic.Int32`) gives per-operation atomicity. Mutexes give multi-operation atomicity across the critical section. If your logic spans more than one access — decrement-then-check, read-then-update, compare-then-store-without-CAS — you need a mutex around the entire sequence.**

This **also surfaces in Rust** — fix is to use `fetch_add`'s **return value** (`let new_rc = rc.fetch_add(1, Ord) + 1;`), not a separate read.

### C. The asymmetric bail (push vs pop)

| Operation | Closed → |
|---|---|
| `push` | Bail immediately. No point pushing. |
| `pop` | Drain remaining items first; bail only when **closed AND empty**. |

Producer-consumer (cv, sem, channel — all three). Get it wrong → silently lose items.

### D. Predicate-loop polarity differs across languages

| Library | API | Wait while...? |
|---|---|---|
| C++ | `cv.wait(lock, pred)` | `pred` is **false** (waits *until* true) |
| Rust | `cv.wait_while(guard, pred)` | `pred` is **true** (waits *while* true) |
| Go | manual `for !pred { cv.Wait() }` | explicit |

**Translating between languages? Read the function name before the body.** Polarity flips between C++ and Rust.

### E. Where shutdown complexity lives

| Primitive | Steady state | Shutdown |
|---|---|---|
| Condvar | predicate loop, retest after wake | one `notify_all` line |
| Semaphore | clean acquire/release | release-N-permits-per-side dance + self-heal |
| Channel (Go) | trivial (`<-`/`<-`/`range`) | "who closes" coordination + can panic |
| Tokio Semaphore | RAII auto-handles release | cross-task lifetimes need `forget()` + `add_permits()` |

> **Choose the primitive that puts complexity in the part of the code you're best at reading. If you can't state the algorithm in English without naming primitives, you haven't understood it yet.**

### F. Correctness vs liveness — the fairness gap

> Concurrency textbook proofs silently assume fair scheduling. Real primitives vary in whether they honor that. **A correctness proof is only as strong as its weakest assumption.**

Manifested concretely in readers-writers turnstile: same algorithm, ~85% pass on `std::counting_semaphore` (no FIFO), 100% on Go channels (FIFO) and tokio Semaphore (FIFO).

### G. Sanitizers are essential, but limited

| Lang | Tool | Build flag |
|---|---|---|
| C++ | TSan | `-DSANITIZER=thread` |
| Rust | TSan (nightly) | `RUSTFLAGS="-Z sanitizer=thread" cargo +nightly run` (recent nightlies need `-Z build-std --target x86_64-unknown-linux-gnu`) |
| Go | race detector | `go run -race .` |

**TSan can detect *structural* deadlock potential. It cannot reason about *quantitative* arguments like "at most N−1 → no full cycle possible."** The footman strategy demonstrates this — TSan flags lock-order-inversion that pigeonhole prevents at runtime.

> **Bar for "done" = 1 000+ stress runs + sanitizer clean. Not "passed once."**

### H. Sabotage experiments are the cheapest learning

Every learnings doc ends with a "30-second sabotage" list. Pattern: edit → predict → run → reflect → revert. Faster than reading; sticks deeper.

Use this technique on any unfamiliar concurrent code:
1. Predict what would break if you remove a specific line.
2. Remove it.
3. Run with sanitizer.
4. Confirm or update your prediction.
5. Revert.

### I. Counter-or-no-counter decision rule

> **A role needs a counter+mutex (lightswitch first-in/last-out) only if it allows *unbounded* concurrent participants. Roles capped at 1 don't — the binary semaphore itself encodes "in / not in."**

Compare across the three roles in §8 (Search-Insert-Delete):

| Role | Max concurrent | Counter? | Why |
|---|---|---|---|
| Searcher | unbounded | **Yes** | Need to know if you're the first to claim or last to yield |
| Inserter | 1 | **No** | The semaphore goes 1→0 on entry, 0→1 on exit; that *is* the count |
| Deleter | 1 | **No** | Same — semaphore is its own occupancy bit |

This generalizes any time you're asked to design synchronization for N roles with mixed concurrency rules: bounded → simple acquire/release, unbounded → lightswitch shape. It's the question to ask **first** when picking primitives for a new variant.

### J. Mutex vs binary semaphore by acquire/release thread

> **Same thread acquires and releases → mutex. Different threads → binary semaphore (permit semantics, no ownership).**

In C++, `std::mutex::unlock()` from a thread that didn't lock it is **undefined behavior**. `std::counting_semaphore<1>::release()` from any thread is fine — semaphores don't track an owner.

This is why the lightswitch's `no_searcher` (or `roomEmpty` in canonical RW) **must** be a `std::counting_semaphore<1>`, not a `std::mutex`: the *first* searcher acquires it, the *last* searcher (or the deleter) releases it. Different threads. Mutex would be UB.

| Primitive | Decision: |
|---|---|
| `mtx_` (counter guard) | **Mutex** — same searcher locks then unlocks within one function |
| `no_searcher` | **Binary semaphore** — first searcher acquires, last searcher (or deleter) releases |
| `no_inserter` | Either works (same thread acquires + releases), but **prefer semaphore** for symmetry |

Worth a 1-sentence mention on any exam answer that uses a binary-semaphore-as-mutex pattern: "We need a semaphore here, not a mutex, because the acquirer and releaser are different threads."

### K. Concurrency rules ARE consistency contracts

> **Each "X may run concurrent with Y" clause in a problem statement implicitly picks a *weaker* consistency contract for the X/Y pair.**

The synchronization machinery enforces the contract; it doesn't *create* it. Recognizing the implied contract tells you what guarantees you owe (and don't owe) the caller.

| Allowed concurrency | Implied contract |
|---|---|
| Reader + reader (no writes) | Trivial; no contract needed |
| Searcher + inserter (SID §8) | "You may miss in-flight inserts; you won't see corruption." Formalized: the call linearizes either before or after the concurrent insert — both legal. |
| Reader + writer (canonical RW: forbidden) | Strong: writes are atomic w.r.t. all reads. Reader sees pre-state or post-state, never mid-state. |
| Deleter excludes everyone (SID §8, RW writer) | Strong: deletes are atomic w.r.t. all observers |
| Producers + consumers via channel (PC §1) | Lossless FIFO over the queue's capacity; the channel itself is the linearization point |

**Exam test:** before designing locks, ask *what consistency contract is each pair of compatible roles giving the caller?* If you can articulate that in one sentence per pair, the locking falls out.

### L. Don't add a flag if existing primitives already encode the state

> **If the state you'd track with a new boolean is already determined by the count/state of existing semaphores or mutexes, don't add the boolean. Two sources of truth drift and bugs hide in the gap.**

In §8 (Search-Insert-Delete), "the deleter is currently active" is fully encoded by `no_searcher.count == 0 && no_inserter.count == 0` — both rooms held. Adding `is_deleter_active = true; ... is_deleter_active = false;` is redundant: it can only *match* what the semaphores already say, or be wrong.

This is the structural cousin of universal lesson **A** (oracle independent of algorithm) — but flipped: in **A**, the oracle deliberately duplicates state for *cross-checking*; here, you're refusing to duplicate state for *control flow*. Both have the same root: don't have two variables both claim to mean the same thing in the algorithm itself.

Symptoms when you've violated this:
- "I set the flag but the semaphore was already at 0" — drift bug.
- "I have to take the mutex just to read the flag" — you've recreated the semaphore's state with extra steps.
- "The flag and the semaphore disagree on shutdown" — you forgot to update one path.

**Default answer: existing primitive state is sufficient. Add a flag only when you genuinely need a *fact* the primitives don't expose** (e.g., "how many searchers are inside" is not what `no_searcher` tells you — for that, you need the explicit counter).

---

## Final checklist before opening any new template

1. **Read the scenario** — get the concrete shape of the abstract problem.
2. **State the algorithm in English.** Name the strategies before typing.
3. **Identify failure modes** to track per problem: deadlock (cycle), livelock (everyone defers, no progress), starvation (one thread consistently late), lost wakeup, lost item.
4. **Pick the primitive** and state where complexity will live (steady state vs shutdown).
5. **Stress + sanitizer.** 100+ runs minimum, with jitter, before declaring done.
6. **Template invariant rules apply:** oracle state is the witness, separate from solution state. Sleep/jitter is intentional — don't remove it to make a flaky run pass.
7. **Per-language defaults:** Go `-race`, Rust nightly TSan with `-Z build-std`, C++ `-DSANITIZER=thread`.
