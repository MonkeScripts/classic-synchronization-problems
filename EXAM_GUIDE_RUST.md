# CS3211 — Rust Exam Book

> **How to use this book during the exam.** Stay in Rust headspace. §1 is the cheat sheet. §2 is the centerpiece — full **program scaffolding** shapes you can write from blank, plus the sync idioms that go inside them. §3 collects classical problems already written in Rust (mostly tokio-async). §4 is intentionally thin: Rust isn't on the 2025 exam, so 4.x just points back at §3 for the analogue patterns. §5–§7 are pitfalls, build commands, and a decision matrix.
>
> **Rust isn't on the 2025 exam.** This book is for self-study; if the exam tests Rust, the same shape applies — read §2 first, then look up the right §3 variant.

---

## Table of contents

0. [Keyword index](#0-keyword-index)
1. [Primitive cheat sheet](#1-primitive-cheat-sheet)
2. [Idioms & scaffolding](#2-idioms--scaffolding) ← **centerpiece**
   - 2a. [Program scaffolding](#2a-program-scaffolding) (S1–S6)
   - 2b. [Sync idioms](#2b-sync-idioms) (I1–I14)
3. [Classical problems in Rust](#3-classical-problems-in-rust)
4. [Exam-style scenarios (stub-with-pointers)](#4-exam-style-scenarios)
5. [Pitfalls catalogue](#5-pitfalls-catalogue)
6. [Build / run / debug](#6-build--run--debug)
7. [Decision matrix](#7-decision-matrix)

---

## 0. Keyword index

| If the question mentions… | Look at |
|---|---|
| shared state across threads | §2a S2 (`Arc<Mutex<T>>`), §2b I1 |
| async / `tokio::spawn` / `.await` | §2a S4, §2b I8 (block-scope guard) |
| FIFO / fairness | §2b I9 (`tokio::sync::Notify`), §3.7 |
| broadcast / pubsub | §2b I14 (`tokio::sync::broadcast`) |
| latest value / watch | §2b I14 (`tokio::sync::watch`) |
| barrier / phase | §2b I6 (semaphore-as-gate), §3.3 |
| condvar / wait_while | §2b I3 (POLARITY INVERSE of C++!) |
| async server / pipelining / fan-out / mpsc | §4.3 (tokio server) |
| BFS / crawler / bounded concurrency / max_workers | §4.4 (bounded crawler from A3) |

---

## 1. Primitive cheat sheet

| Primitive | Module | Async? | What it gives you |
|---|---|---|---|
| `std::sync::Mutex<T>` | `std::sync` | no | Mutex with poisoning. RAII guard. **`MutexGuard` is `!Send`** — can't be held across `.await`. |
| `std::sync::RwLock<T>` | `std::sync` | no | Reader-writer lock. RAII guards. |
| `std::sync::Condvar` | `std::sync` | no | Parking + notification. **`wait_while` predicate API — polarity OPPOSITE C++.** |
| `std::sync::atomic::AtomicI32`, `AtomicBool` | `std::sync::atomic` | no | Per-op atomicity. **Every op requires explicit `Ordering`** — no defaults. |
| `Arc<T>` | `std::sync` | no | Atomically reference-counted shared pointer. |
| `tokio::sync::Mutex<T>` | `tokio::sync` | yes | Async-aware mutex. Use ONLY when guard held across `.await`. |
| `tokio::sync::Semaphore` | `tokio::sync` | yes | Counting semaphore with **FIFO wakeup**. Permits are RAII. |
| `tokio::sync::Barrier` | `tokio::sync` | yes | Reusable phase synchronization. |
| `tokio::sync::Notify` | `tokio::sync` | yes | Wakeup signal. `notify_one()` deposits a permit (memory!); `notify_waiters()` wakes currently parked only. |
| `tokio::sync::oneshot` | `tokio::sync` | yes | Single-shot channel. |
| `tokio::sync::mpsc` | `tokio::sync` | yes | Async multi-producer, single-consumer. |
| `tokio::sync::broadcast` | `tokio::sync` | yes | Pubsub fan-out (lossy ring buffer). |
| `tokio::sync::watch` | `tokio::sync` | yes | Latest-value broadcast (Receiver IS Clone). |
| `std::sync::mpsc` | `std::sync` | no | Sync mpsc. **`Receiver` is NOT `Clone`** — wrap in `Arc<Mutex<Receiver>>` for multi-consumer. |

---

## 2. Idioms & scaffolding

### 2a. Program scaffolding

#### S1. Sync-only scaffold (`std::thread` + `Arc<Mutex<T>>`)

```rust
use std::sync::{Arc, Mutex};
use std::thread;

fn main() {
    let shared = Arc::new(Mutex::new(0));

    let mut handles = vec![];
    for _ in 0..4 {
        let shared = Arc::clone(&shared);                  // *** clone PER spawn ***
        handles.push(thread::spawn(move || {
            let mut g = shared.lock().unwrap();             // named binding, ALWAYS
            *g += 1;
        }));
    }
    for h in handles { h.join().unwrap(); }
    println!("{}", *shared.lock().unwrap());
}
```

**Critical invariants:**
- **Clone the Arc PER spawn** before the closure. The `move` consumes the closure's captures; if you don't clone first, the second iteration sees use-of-moved-value.
- **`thread::spawn(move || ...)`** captures by move — required because the thread outlives the spawning frame.
- **`.join()` returns `Result<T, _>`** — `.unwrap()` to propagate panics from the joined thread.

#### S2. Async scaffold (`#[tokio::main]` + `tokio::spawn`)

```rust
use std::sync::Arc;
use tokio::sync::Mutex;                                    // tokio Mutex if held across .await

#[tokio::main]
async fn main() {
    let shared = Arc::new(Mutex::new(0_i32));

    let mut handles = vec![];
    for _ in 0..4 {
        let shared = Arc::clone(&shared);
        handles.push(tokio::spawn(async move {
            let mut g = shared.lock().await;                // .await on tokio Mutex
            *g += 1;
        }));
    }
    for h in handles { h.await.unwrap(); }
    println!("{}", *shared.lock().await);
}
```

**Cargo.toml.**
```toml
[package]
name = "exam"
version = "0.1.0"
edition = "2021"

[dependencies]
tokio = { version = "1", features = ["full"] }              # "rt-multi-thread", "macros", "sync", "time"
```

#### S3. Class-shaped struct + `impl new`

```rust
pub struct Buffer<T> {
    inner: std::sync::Mutex<BufferInner<T>>,
    not_full:  std::sync::Condvar,
    not_empty: std::sync::Condvar,
    capacity: usize,
}

struct BufferInner<T> {
    queue: std::collections::VecDeque<T>,
    closed: bool,                                            // *** lives BEHIND the lock ***
}

impl<T> Buffer<T> {
    pub fn new(cap: usize) -> Self {
        Self {
            inner: std::sync::Mutex::new(BufferInner {
                queue: std::collections::VecDeque::new(),
                closed: false,
            }),
            not_full:  std::sync::Condvar::new(),
            not_empty: std::sync::Condvar::new(),
            capacity:  cap,
        }
    }

    pub fn push(&self, item: T) -> Result<(), T> { /* ... */ todo!() }
    pub fn pop(&self) -> Option<T> { /* ... */ todo!() }
    pub fn close(&self) { /* ... */ }
}
```

**Shape rules.**
- **State that lives behind the mutex goes in an inner struct.** `inner.queue`, never `self.queue`. The mutex wraps one struct; locking gives you a guard that derefs to that struct.
- **Public methods take `&self`** (not `&mut self`). The `Mutex` provides interior mutability.
- **Constructor returns `Self`**, not `Arc<Self>` — let the caller wrap with `Arc::new(Buffer::new(...))`.

#### S4. Tokio task spawn pattern with Arc clone-per-spawn

```rust
let factory = Arc::new(WaterFactory::new());
let mut handles = Vec::new();

for i in 0..N_HYDROGEN {
    let factory = Arc::clone(&factory);                     // *** outside async move ***
    handles.push(tokio::spawn(async move {
        factory.hydrogen(i).await;
    }));
}
for h in handles { h.await.unwrap(); }
```

**Critical:** the `Arc::clone` MUST happen outside the `async move`. Inside the closure, `factory` has already been moved on the first iteration; `Arc::clone(&factory)` inside fails to compile on iteration 2 with "use of moved value: `factory`."

#### S5. Graceful shutdown — `tokio::select!` + cancellation token

```rust
use tokio::select;

async fn worker(
    work_rx: &mut tokio::sync::mpsc::Receiver<Task>,
    cancel: tokio::sync::watch::Receiver<bool>,
) {
    loop {
        let mut cancel_rx = cancel.clone();
        select! {
            biased;                                          // optional: deterministic order
            _ = cancel_rx.changed() => {
                if *cancel_rx.borrow() { return; }
            }
            Some(task) = work_rx.recv() => {
                process(task).await;
            }
            else => return,                                  // both branches finished
        }
    }
}
```

For the simpler "drop the senders and let mpsc close" pattern, see I11 + the H₂O daemon example in §3.6.

#### S6. The "tokio mpsc as mailbox" daemon shape

When the question says "centralized coordinator," the daemon owns a `mpsc::Receiver`; clients hold cloned `mpsc::Sender`s.

```rust
use tokio::sync::{mpsc, oneshot};

pub struct Service {
    req_tx: mpsc::Sender<Request>,
}

struct Request {
    payload: String,
    reply: oneshot::Sender<String>,
}

impl Service {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel(32);
        tokio::spawn(daemon(rx));                            // *** spawn at construction ***
        Self { req_tx: tx }
    }

    pub async fn call(&self, payload: String) -> String {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.req_tx
            .send(Request { payload, reply: reply_tx }).await
            .unwrap();
        reply_rx.await.unwrap()
    }
}

async fn daemon(mut rx: mpsc::Receiver<Request>) {
    while let Some(r) = rx.recv().await {                    // *** Some/None, not Result ***
        let result = process(&r.payload).await;
        let _ = r.reply.send(result);                        // ignore Err if caller cancelled
    }
    // rx returned None → all senders dropped → shutdown
}
```

---

### 2b. Sync idioms

#### I1. `Arc<Mutex<T>>` — spatial sharing + temporal exclusion

**When.** Default for any "share mutable state across threads."

```rust
let shared = Arc::new(Mutex::new(state));

for _ in 0..N {
    let shared = Arc::clone(&shared);
    thread::spawn(move || {
        let mut g = shared.lock().unwrap();                  // *** named binding ***
        g.field += 1;                                         // mutate THROUGH guard
    });
}
```

**Gotchas.**
- **`Arc<T>` only hands out `&T`.** To MUTATE, the inner type needs interior mutability (`Mutex`, `RwLock`, `AtomicI32`).
- **Clone the Arc per spawn** — `move` consumes the closure's captures.
- **`let _ = m.lock();`** triggers `let_underscore_lock` lint — drops guard immediately. Use `let _g = ...` (named binding with leading `_`).

#### I2. Operators on a guard need explicit deref

```rust
let mut g = m.lock().unwrap();

g.method()                                                   // auto-deref OK
g.field += 1                                                 // auto-deref OK
*g == X                                                      // *** explicit deref needed ***
*g += 1                                                      // operator on outer type fails without *
*g < CAPACITY                                                // same
```

**Gotcha.** Auto-deref through smart pointers fires for methods/field access only — NOT operators. `*` belongs at the use site, not the binding (`let mut *g = ...` is a syntax error).

#### I3. Condvar `wait_while` (POLARITY OPPOSITE C++)

```rust
use std::sync::{Mutex, Condvar};

// Idiomatic shape — SHADOW the binding with the guard returned by wait_while.
let state = self.state.lock().unwrap();                      // acquire
let mut state = self.cv
    .wait_while(state, |s| !pred(s))                         // *** wait WHILE pred true ***
    .unwrap();
state.something += 1;
```

**The use-after-move trap (most common failure):**
```rust
// WRONG — wait_while CONSUMES `shared`. Binding to `_guard` while trying to
// keep using `shared` fails with "use of moved value."
let mut shared = self.state.lock().unwrap();
let _guard = self.cv.wait_while(shared, |s| s.draining).unwrap();
shared.seated += 1;                                          // ← compile error

// RIGHT — shadow `shared` with the new guard:
let shared = self.state.lock().unwrap();
let mut shared = self.cv.wait_while(shared, |s| s.draining).unwrap();
shared.seated += 1;                                          // ✓
```

**Gotchas.**
- **Polarity OPPOSITE C++.** C++ `wait(lock, pred)` waits *until* true; Rust `wait_while(g, pred)` waits *while* true. Always read the function name before the body.
- **`std::sync::Mutex` is NOT reentrant.** `wait_while` consumes the guard you already hold; don't re-lock from inside the predicate.
- **`wait_while` returns `LockResult<MutexGuard>`.** Always `.unwrap()`; forgetting it gives you a `LockResult`, not a guard, and the next field access fails to compile.
- **Closure parameter shadows captures.** Pick a non-colliding name (`s`, `g`).
- **Booleans in the predicate are lowercase: `s.draining`.** `True`/`False` (capitalized) are undefined.

##### Sub-pattern: `notify_one` vs `notify_all`

> **Rule of thumb: if more than one parked waiter could pass the predicate after your update, use `notify_all`. Otherwise `notify_one` is sufficient.**

| Scenario | Why | Use |
|---|---|---|
| Producer-consumer: `push` adds 1 item | Only 1 consumer's predicate (`!empty`) can be satisfied | `notify_one` |
| Producer-consumer: `pop` frees 1 slot | Only 1 producer's predicate (`!full`) can be satisfied | `notify_one` |
| Sushi bar drain ends (`draining = false`) | **Up to N parked customers** can now pass `!draining` | **`notify_all`** |
| Barrier reaches threshold | All N-1 parked threads can proceed | **`notify_all`** |
| RW writer finishes | All parked readers can proceed | **`notify_all`** |
| Single-resource handoff (1 token, 1 winner) | Only 1 waiter will succeed | `notify_one` |

**When unsure, prefer `notify_all`.** Correctness > efficiency under exam pressure.

#### I4. Atomic compound-op via `fetch_*` return value

```rust
use std::sync::atomic::{AtomicI32, Ordering};

let new_rc = self.rc.fetch_add(1, Ordering::SeqCst) + 1;
if new_rc == 1 {
    // first-reader path
}

let new_rc = self.rc.fetch_sub(1, Ordering::SeqCst) - 1;
if new_rc == 0 {
    // last-reader path
}
```

**Gotchas.**
- DON'T do `fetch_add` then a separate `load` — that's a compound-op race. The return value of `fetch_add` is the *previous* value; add/subtract to get current.
- **Per-op atomicity ≠ multi-op atomicity.** For arbitrary compound logic, wrap in a mutex.
- **Rust requires explicit `Ordering`** — no default. `Ordering::SeqCst` is the right starting point.

#### I5. Tokio Semaphore — RAII pool

**When.** Resource pool, rate limit, footman. Permit auto-returns on Drop.

```rust
use tokio::sync::Semaphore;

let pool = Semaphore::new(capacity);

// in each task:
let _permit = pool.acquire().await.unwrap();                 // hold while in scope
// ... do work ...
// _permit drops at end of scope → permit auto-returns
```

**Gotchas.**
- Permits are RAII. **Never call `add_permits` again** for a pool — count just oscillates between 0 and capacity.
- Even if a task panics, Drop runs on unwind; the permit is freed.
- **No `.release()` method exists.** Permits return on Drop.

#### I6. Tokio Semaphore — gate / one-shot signal (`forget()`)

**When.** Barrier-style broadcast — tickets are *consumed* on use, not refunded.

```rust
let gate = Semaphore::new(0);                                // gate closed

// last arriver opens the gate:
gate.add_permits(N);

// each thread walks through:
gate.acquire().await.unwrap().forget();                      // *** forget() consumes ***
```

**Gotchas.**
- WITHOUT `.forget()`: every acquired permit auto-returns on Drop → gate refills → round 2 races through.
- Pool ↔ Drop, gate ↔ `.forget()`. Same primitive, opposite semantics.
- For the reciprocal — first-acquirer / last-releaser in DIFFERENT tasks — see I7.

#### I7. Cross-task permit lifetime (`forget` + `add_permits`)

**When.** First-acquirer and last-releaser are in DIFFERENT tasks (readers-writers first/last reader).

```rust
// Task A — first reader:
let permit = self.room_empty.acquire().await.unwrap();
permit.forget();                                              // skip Drop; sem stays decremented

// Task B (later) — last reader:
self.room_empty.add_permits(1);                               // manually return one
```

**Gotchas.**
- RAII can't bridge tasks. `forget()` + `add_permits()` is the explicit escape hatch.
- Every `forget` MUST be matched by an `add_permits(1)` — otherwise capacity leaks permanently.

#### I8. Block-scope guard before `.await`

**When.** Inside an `async fn` that will be `tokio::spawn`-ed; need a `std::sync::Mutex`.

```rust
let result = {
    let g = self.map.lock().unwrap();                         // !Send guard
    g.get(k).cloned().unwrap_or_default()
};   // *** g lexically dead HERE — drop(g) does NOT help ***

do_async_thing(result).await;
```

**Gotchas.**
- **`std::sync::MutexGuard` is `!Send`.** Holding one across `.await` makes the future `!Send` → `tokio::spawn` rejects with "future is not `Send`."
- **`drop(guard)` does NOT satisfy the analysis.** Send-auto-trait analysis is **lexical**, not dataflow.
- Use block-scope `{ ... }` so the guard is structurally out of scope before the await.
- `tokio::sync::Mutex` is the alternative — only when you genuinely need to hold the lock across `.await`.

#### I9. Tokio `Notify` — wakeup with deposit-permit memory

**When.** Replace condvar; signaler may fire before waiter parks.

```rust
use tokio::sync::Notify;
use std::sync::Arc;

let notify = Arc::new(Notify::new());

// signaler:
notify.notify_one();                                          // deposits a permit if no waiter currently parked

// waiter (later, even if signaler fired earlier):
notify.notified().await;                                      // consumes the deposited permit
```

**Broadcast variant:**
```rust
notify.notify_waiters();                                      // wakes ALL CURRENTLY parked; does NOT deposit
```

**Gotchas.**
- `notify_one` deposits a permit (memory). `notify_waiters` does NOT — only wakes currently parked.
- For predicate-loop equivalent, you need `Mutex<State> + Notify` and a manual loop.

#### I10. Tokio `oneshot` — addressed single-shot signal

**When.** Send exactly one message to one specific receiver.

```rust
use tokio::sync::oneshot;

let (tx, rx) = oneshot::channel::<()>();

// signaler (typically passes tx to whoever produces the value):
tx.send(()).unwrap();                                         // returns Err if Receiver dropped

// awaiter:
rx.await.unwrap();                                            // returns Err if Sender dropped without sending
```

**Gotchas.**
- **Single-shot** — once value goes through, channel is dead. For multi-message use mpsc.
- `Sender::send` is sync (returns `Result<(), T>`). Receiver is async (`.await`).
- For "release-style" semantics where you don't care if receiver dropped: `let _ = tx.send(());`.

#### I11. Tokio `mpsc` — async multi-producer, single-consumer

```rust
use tokio::sync::mpsc;

let (tx, mut rx) = mpsc::channel::<T>(BUFFER);

// each producer (Sender is Clone):
for _ in 0..N {
    let tx = tx.clone();
    tokio::spawn(async move {
        tx.send(x).await.unwrap();                            // awaits when full — backpressure
    });
}
drop(tx);                                                     // *** main's clone dropped → channel can close ***

// consumer:
while let Some(x) = rx.recv().await {                         // *** Some/None — NOT Result ***
    process(x);
}
```

**Gotchas.**
- **`tokio::mpsc::Receiver::recv()` returns `Option<T>`.** `std::sync::mpsc::Receiver::recv()` returns `Result<T, RecvError>`. Different sentinels — easy to confuse.
- **Channel closes when LAST sender drops.** Main holds an unused `tx` clone? Receiver hangs. Always `drop(tx)` after spawning producers.
- **Sender is Clone; Receiver is NOT.** For multi-consumer, see I12.
- `tx.send(x).await` awaits when full — that IS the right async backpressure idiom. Avoid `try_send` + sleep unless you specifically want to bail.

#### I12. Tokio `mpsc` + `Arc<Mutex<Receiver>>` — multi-consumer

```rust
use tokio::sync::{mpsc, Mutex};
use std::sync::Arc;

let (tx, rx) = mpsc::channel::<String>(BUFFER_SIZE);
let recv_lock = Arc::new(Mutex::new(rx));

// producers — clone tx
for pid in 0..NUM_PRODUCERS {
    let tx = tx.clone();
    tokio::spawn(async move {
        tx.send(format!("P{}", pid)).await.unwrap();
    });
}
drop(tx);

// consumers — share Arc<Mutex<Receiver>>
for _ in 0..NUM_CONSUMERS {
    let recv_lock = Arc::clone(&recv_lock);
    tokio::spawn(async move {
        loop {
            let item = {                                       // *** tight lock scope ***
                let mut rx = recv_lock.lock().await;
                rx.recv().await
            };  // guard dropped HERE, before match
            match item {
                Some(msg) => { /* ... */ }
                None => break,
            }
        }
    });
}
```

**Gotchas.**
- `recv_lock` uses `tokio::sync::Mutex` (not `std`) because we hold across `.await`.
- **Tight-scope the lock** so `recv().await` doesn't serialize *waiting* across consumers. The block `{ let mut rx = lock.lock().await; rx.recv().await }` releases the mutex as soon as `recv` returns.
- Performance caveat: this is *effectively serial*. `crossbeam-channel`'s `Receiver` is `Clone` and `Sync` and gives true concurrent multi-consumer.

#### I13. Clone-before-move in `tokio::spawn` loops

```rust
// WRONG — `done_tx` moves into the FIRST iteration's closure;
//         the second iteration sees use-of-moved-value.
for i in 0..N {
    handles.push(tokio::spawn(async move {
        worker(i, done_tx.clone()).await;                     // .clone() inside is too late
    }));
}

// RIGHT — clone OUTSIDE, move the clone INTO the closure.
for i in 0..N {
    let dt = done_tx.clone();                                 // *** outside async move ***
    handles.push(tokio::spawn(async move {
        worker(i, dt).await;                                  // closure captures dt only
    }));
}
drop(done_tx);                                                // drop main's instance
```

**Gotchas.**
- `async move` captures variables by move — once moved, the original binding is gone.
- The `.clone()` *inside* the closure body is irrelevant — by then `done_tx` has already been moved.
- Same pattern with `Arc<T>`: `let a = Arc::clone(&arc); spawn(async move { use(a) });`.

#### I14. Tokio `broadcast` and `watch` — pubsub fan-out

| Primitive | Semantic | Receiver fan-out | Slow consumer | Capacity |
|---|---|---|---|---|
| `broadcast` | "every event" | `tx.subscribe()` | `Lagged(n)` — must `continue`, not `break` | ring buffer |
| `watch` | "latest value" | `rx.clone()` | silently skips intermediates | always 1 |

```rust
// broadcast — pubsub
use tokio::sync::broadcast;
let (tx, _rx) = broadcast::channel::<i32>(5);
let mut rx_c = tx.subscribe();
tokio::spawn(async move {
    loop {
        match rx_c.recv().await {
            Ok(msg) => { /* ... */ }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                eprintln!("lagged, missed {n}");
                continue;                                      // *** NEVER break on Lagged ***
            }
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
});

// watch — latest-value
use tokio::sync::watch;
let (tx, rx) = watch::channel(initial);
let mut rx = rx.clone();
tokio::spawn(async move {
    loop {
        { let snap = rx.borrow(); /* read */ }                 // drop borrow before .await
        if rx.changed().await.is_err() { break; }
    }
});
```

**Gotchas.**
- **Don't `break` on `Lagged(n)`.** Recoverable; `continue` to skip the gap. A subscriber that breaks commits suicide.
- **Don't hold `rx.borrow()` across `.await`.** It's a sync lock against the publisher's `send`.

---

## 3. Classical problems in Rust

Each problem composes §2 idioms. For algorithmic depth, see `EXAM_GUIDE_PER_PROBLEM.md`.

### 3.1 Producer–consumer

#### Variant A — sync condvar

Composes **S3** + **I3**.

```rust
struct BufferInner<T> {
    queue: VecDeque<T>,
    closed: bool,
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
            inner = self.not_full.wait(inner).unwrap();
        }
        if inner.closed { return Err(item); }                  // give back item, not bool
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

#### Variant B — async tokio mpsc

Composes **I11** + **I12** for multi-consumer.

See I12 for the full multi-consumer shape; for single-consumer, drop the `Arc<Mutex<Receiver>>` wrap.

**Mistakes worth memorizing.**
1. `std::move(item)` — Rust moves implicitly.
2. `Ok()` instead of `Ok(())` — unit `()` is still a value.
3. `if cond { Err(item) }` then fall-through — `if` with no `else` evaluates to `()`.
4. `self.closed` instead of `inner.closed` — state lives behind the lock.
5. `inner.pop_front()` (where `inner` is the guard) — `pop_front` is on `inner.queue`, not on the guard.

---

### 3.2 Readers–writers (async tokio)

Composes **S2** + **I5** + **I7** + **I8**.

```rust
use tokio::sync::Semaphore;
use std::sync::{Mutex, atomic::{AtomicI32, Ordering}};

pub struct KVCache {
    map: Mutex<HashMap<String, String>>,                       // std — no .await across
    turnstile: Semaphore,
    room_empty: Semaphore,
    bibo: Semaphore,
    rc: AtomicI32,                                              // mutated under bibo
}

impl KVCache {
    pub async fn get(&self, k: &str) -> String {
        drop(self.turnstile.acquire().await.unwrap());          // gate-pass

        {
            let _bibo = self.bibo.acquire().await.unwrap();
            let new_rc = self.rc.fetch_add(1, Ordering::SeqCst) + 1;
            if new_rc == 1 {
                let permit = self.room_empty.acquire().await.unwrap();
                permit.forget();                                 // cross-task lifetime
            }
        }

        let out = {                                              // tight scope
            let g = self.map.lock().unwrap();
            g.get(k).cloned().unwrap_or_default()
        };  // guard MUST be lexically dead before next .await

        {
            let _bibo = self.bibo.acquire().await.unwrap();
            let new_rc = self.rc.fetch_sub(1, Ordering::SeqCst) - 1;
            if new_rc == 0 {
                self.room_empty.add_permits(1);                  // reciprocal of forget
            }
        }
        out
    }

    pub async fn set(&self, k: String, v: String) {
        let _turnstile = self.turnstile.acquire().await.unwrap();   // LIFO drop
        let _room      = self.room_empty.acquire().await.unwrap();
        { let mut g = self.map.lock().unwrap(); g.insert(k, v); }
    }
}
```

**Mistakes worth memorizing.**
1. `.release()` on tokio Semaphore — doesn't exist. Permits are RAII; `forget` + `add_permits` for cross-task.
2. `.acquire()` without `.await` — returns an unstarted Future; nothing happens.
3. Compound-op race on `self.rc` — use `fetch_add` return value, not separate load.
4. `MutexGuard` held across `.await` — "future is not Send" compile error.
5. `drop(guard); something.await;` — Send analysis is lexical, not dataflow. Wrap in `{ ... }` block instead.

---

### 3.3 Barrier

#### Sync variant — Mutex + Condvar + generation

Composes **S3** + **I3**.

```rust
struct BarrierState { count: usize, generation: u64 }

pub struct MyBarrier { expected: usize, state: Mutex<BarrierState>, cv: Condvar }

impl MyBarrier {
    pub fn new(n: usize) -> Self {
        Self { expected: n, state: Mutex::new(BarrierState { count: 0, generation: 0 }), cv: Condvar::new() }
    }

    pub fn arrive_and_wait(&self) {
        let mut state = self.state.lock().unwrap();
        let gen = state.generation;
        state.count += 1;
        if state.count == self.expected {
            state.count = 0;
            state.generation += 1;
            self.cv.notify_all();
            return;
        }
        let _guard = self.cv
            .wait_while(state, |s| gen == s.generation)            // wait WHILE pred true
            .unwrap();
    }
}
```

#### Async variant — tokio Semaphore preloaded turnstile

Composes **I6**.

```rust
async fn arrive_and_wait(&self) {
    // ... last arriver: self.t1.add_permits(self.expected);
    self.t1.acquire().await.unwrap().forget();                     // *** forget MANDATORY ***
    // ... last arriver: self.t2.add_permits(self.expected);
    self.t2.acquire().await.unwrap().forget();
}
```

**Mistakes worth memorizing.**
1. Predicate polarity inversion when porting from C++. C++ waits *until* true, Rust `wait_while` waits *while* true.
2. `let _ = wait_while(...)` triggers `let_underscore_lock` lint — drops guard immediately.
3. `tokio::Barrier` calling without `.await` — future created and immediately dropped.
4. Forgetting `.forget()` on the preloaded turnstile — permits auto-return on Drop, gate refills.

---

### 3.4 Dining philosophers (tokio asymmetric)

```rust
use tokio::sync::Mutex;

pub struct Table { chopsticks: Vec<Mutex<()>> }

impl Table {
    pub fn new(n: usize) -> Self {
        Self { chopsticks: (0..n).map(|_| Mutex::new(())).collect() }
    }

    pub async fn eat(&self, pid: usize) {
        let n = self.chopsticks.len();
        let (left, right) = (pid, (pid + 1) % n);
        let (a, b) = if pid == n - 1 { (right, left) } else { (left, right) };  // ASYMMETRIC
        let _g1 = self.chopsticks[a].lock().await;
        let _g2 = self.chopsticks[b].lock().await;
        // EATING — guards drop LIFO at scope end
    }
}
```

**Why tokio Mutex (not std).** `_g1` is held across `.await` on the next lock. `std::sync::MutexGuard` is `!Send` — the future would be rejected by `tokio::spawn`.

---

### 3.5 Barbershop

Composes **I5** + **I6** (signal-style with `add_permits` + `forget`).

```rust
pub struct Barbershop {
    customers: Mutex<usize>,                                       // std::sync
    customer_sem: Semaphore,
    barber_sem: Semaphore,
    customer_done: Semaphore,
    barber_done: Semaphore,
    stop: AtomicBool,
}

impl Barbershop {
    pub async fn customer(&self, _id: i32) -> bool {
        {
            let mut n = self.customers.lock().unwrap();
            if *n == CHAIRS { return false; }                      // *guard for ops!
            *n += 1;
        }
        self.customer_sem.add_permits(1);                          // V
        self.barber_sem.acquire().await.unwrap().forget();         // P
        tokio::time::sleep(Duration::from_millis(2)).await;
        self.customer_done.add_permits(1);                          // V
        self.barber_done.acquire().await.unwrap().forget();         // P
        { let mut n = self.customers.lock().unwrap(); *n -= 1; }
        true
    }

    pub async fn barber(&self) {
        loop {
            self.customer_sem.acquire().await.unwrap().forget();
            if self.stop.load(Ordering::SeqCst) { break; }          // RECHECK
            self.barber_sem.add_permits(1);
            tokio::time::sleep(Duration::from_millis(2)).await;
            self.barber_done.add_permits(1);
            self.customer_done.acquire().await.unwrap().forget();
        }
    }
}
```

**Alternative — channel-based barbershop with `mpsc<oneshot::Sender<()>>`:**
```rust
pub struct Barbershop { tx: mpsc::Sender<oneshot::Sender<()>> }
// try_send(done_tx) → Ok = sat, Err(Full) = balk; barber recv-then-cut-then-done_tx.send(()).
```

The mpsc channel of `oneshot::Sender<()>` collapses both the chair-count AND the front handshake. `try_send` "into" the buffer = sit; `Err(Full)` = balk. The oneshot IS the back handshake.

---

### 3.6 H₂O (water factory) — daemon strategy

Composes **S6** + **I10** + **I11**.

```rust
struct Request {
    go: oneshot::Sender<()>,                                       // daemon → atom
    done: oneshot::Receiver<()>,                                   // atom → daemon
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

**Why TWO oneshots per atom (vs Go's single channel).** Go reuses one bidirectional `chan struct{}` for both go and done. Rust's `oneshot` is **single-shot** — once the value goes through, the channel is dead. So you need two oneshots per atom: go (daemon→atom), done (atom→daemon).

---

### 3.7 FIFO semaphore — oneshot queue

Composes **S3** + **I8** + **I10**.

```rust
struct State {
    permits: usize,
    waiters: VecDeque<oneshot::Sender<()>>,
}

pub struct FifoSemaphore { state: std::sync::Mutex<State> }

impl FifoSemaphore {
    pub fn new(initial: usize) -> Self {
        Self { state: Mutex::new(State { permits: initial, waiters: VecDeque::new() }) }
    }

    pub async fn acquire(&self) {
        let rx = {
            let mut s = self.state.lock().unwrap();
            if s.permits > 0 && s.waiters.is_empty() {
                s.permits -= 1;
                return;                                             // fast path
            }
            let (tx, rx) = oneshot::channel();
            s.waiters.push_back(tx);
            rx
        };                                                          // *** drop std::Mutex guard before await ***
        rx.await.unwrap();
    }

    pub fn release(&self) {
        let mut s = self.state.lock().unwrap();
        if let Some(tx) = s.waiters.pop_front() {
            drop(s);
            let _ = tx.send(());                                    // direct hand-off
        } else {
            s.permits += 1;
        }
    }
}
```

**Why `std::sync::Mutex`.** Critical sections are short and don't span `.await` (the block-scope releases the guard before `rx.await`). `std::Mutex` is faster.

**Why direct hand-off, not "decrement-and-bump."** On release with a waiter present, `tx.send(())` directly hands the permit to the front-of-queue waiter — never increment `permits` and let the next acquirer grab it (that breaks FIFO under contention).

---

### 3.8 Search-Insert-Delete

Composes **I5** + **I7** (tokio Semaphore in pool/gate roles).

The shape mirrors §3.2 readers-writers but with three roles. See `EXAM_GUIDE_PER_PROBLEM.md` §8 for the full discussion; the Rust idiom is:

```rust
pub struct SidList {
    bibo: Mutex<()>,                                                // guards searcher_count
    searcher_count: AtomicI32,
    no_searcher: Semaphore,                                         // cap 1
    no_inserter: Semaphore,                                         // cap 1
    // ... data with atomic-size publication ...
}

// search: bibo, count++, if first acquire no_searcher (forget), bibo
// insert: acquire no_inserter (RAII drop)
// delete: acquire no_searcher (forget), acquire no_inserter (RAII drop), do work, add_permits(no_searcher)
```

---

## 4. Exam-style scenarios

> Rust isn't on the 2025 paper. The shapes below are stub-with-pointers — they reduce each scenario to the §2 idioms and the §3 classical problem they map to.

### 4.1 TicketSystem analogue (Rust port of 2025 Q25 / Q26)

**Mutex+state version:** §2a S3 (struct + impl) + §3.5 (worker thread idiom — spawn a tokio task that periodically sweeps, owned by the struct, cancelled on Drop).

**Atomic-only (CAS) version:** §2b I4 — `seat.available.compare_exchange(true, false, AcqRel, Acquire)` mirrors the C++ pattern exactly. No mutex; `Arc<Vec<Seat>>` shared across tasks.

### 4.2 LoadBalancer analogue (Rust port of 2025 Q27)

**Channel-only version:** §2a S6 + §3.6 (daemon shape). N tokio mpsc channels (one per server), an `Arc<LoadBalancer>` shared across spawn, `Dispatch(req)` picks `serverIdx` by hash and `tx[idx].send(req).await`. Server task `recv`s in a loop; reply via per-request `oneshot::Sender`.

**Graceful shutdown analogue (Q28):** drop the per-server `tx` clones in main; each server's `rx.recv().await` returns `None` after all senders drop → server exits the loop → join handles. Same shape as I11.

---

### 4.3 Rust async server with tokio mpsc pipelining (2023 Q9)

**Scenario.** A Rust server using tokio for async I/O. Submit queue (SQ) is an mpsc channel; the server reads requests from clients, places them in SQ, calls async `process()` on each, places the result in CQ, and writes back to the client. Maximum concurrency = the channel capacities.

**This is the 3-stage async pipeline (read → process → write) using tokio mpsc channels.** Composes **S4** (tokio task spawn) + **I11** (mpsc) + fan-out at process / fan-in at CQ.

```rust
use tokio::sync::mpsc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

enum IoOperation {
    Read(tokio::net::TcpStream, Vec<u8>),
    Write(tokio::net::TcpStream, Vec<u8>),
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let listener = TcpListener::bind("0.0.0.0:8080").await?;
    let (sq_tx, mut sq_rx) = mpsc::channel::<IoOperation>(SIZE);
    let (cq_tx, mut cq_rx) = mpsc::channel::<IoOperation>(SIZE);

    loop {
        let (stream, _) = listener.accept().await?;

        // Per-client read task — submits READs to SQ
        let sq_tx_clone = sq_tx.clone();
        tokio::spawn(async move {
            let mut stream = stream;
            loop {
                let mut buf = Vec::new();
                if stream.read(&mut buf).await.is_err() { break; }
                if sq_tx_clone.send(IoOperation::Read(stream, buf)).await.is_err() { break; }
                // (in real code, stream is consumed by the Read; for the rubric shape it's stylized)
            }
        });

        // Process worker — single mpsc consumer (fan-out via tokio::spawn per item)
        let cq_tx_clone = cq_tx.clone();
        tokio::spawn(async move {
            while let Some(op) = sq_rx.recv().await {
                let cq_tx = cq_tx_clone.clone();
                tokio::spawn(async move {
                    if let IoOperation::Read(stream, data) = op {
                        let res = process(data).await;
                        let _ = cq_tx.send(IoOperation::Write(stream, res)).await;
                    }
                });
            }
        });

        // Send worker — drains CQ and writes back to clients
        tokio::spawn(async move {
            while let Some(op) = cq_rx.recv().await {
                if let IoOperation::Write(mut stream, data) = op {
                    let _ = stream.write_all(&data).await;
                }
            }
        });
    }
}
```

**Concurrency analysis (rubric).**
- **General overview:** for each client, spawn one tokio task for handling incoming requests and one for sending back results. Only **one** tokio task can retrieve from SQ (mpsc has a single consumer); inside that task, spawn-per-item gives you parallel processing.
- **Programming paradigms:** pipelining (read → process → write); fan-out (process spawns a sub-task per item); fan-in (sub-tasks send to a single CQ).
- **Maximum parallelism:** the maximum size of the channels (SIZE) — backpressure throttles the system once SQ is full.

**Patterns reused.** S4 (spawn at construction); I11 (mpsc — single-consumer means we must spawn-per-item to parallelize processing); I13 (clone-before-move in spawn loops).

**Why mpsc + spawn-per-item, not multi-consumer mpsc.** `tokio::mpsc::Receiver` is single-consumer. To parallelize processing you have ONE consumer task that pulls items, then `tokio::spawn`s a sub-task per item. The runtime schedules sub-tasks across worker threads. For true concurrent receivers, you'd need `Arc<Mutex<Receiver>>` (I12) — but that serializes the *waiting*, defeating the purpose. The spawn-per-item shape is the canonical async fan-out.

**Common mistakes.**
- Forgetting `.await` on `sq_tx.send(...)` — returns a Future that does nothing if dropped. The send doesn't happen.
- `tx.send(value).unwrap()` — panics if the receiver was dropped (channel closed). Use `let _ = tx.send(value).await;` for release-style semantics.
- Trying to `Receiver::clone()` — doesn't compile. Use `Arc<Mutex<Receiver>>` (I12) or restructure to single-consumer + spawn-per-item.
- Missing `tokio = { version = "1", features = ["full"] }` in `Cargo.toml` — `tokio::main` macro isn't available without `macros`; tcp/io isn't available without `net`/`io-util`.

---

### 4.4 From Assignment 3 — bounded-concurrency BFS crawler (real implementation reference)

**Scenario.** Real CS3211 assignment: a Rust async URL crawler. Performs BFS on a fake web server starting from `/hub`, respects a max depth, never fetches the same URL twice, bounds peak concurrency to `max_workers` regardless of how many tokio worker threads exist.

#### Architecture

```
Level 0:                /hub                    (1 page)
                          │
Level 1:    ┌─────────┬───┴───┬─────────┐      (fan-out)
                                                 │ each gated by Arc<Semaphore>(max_workers)
Level 2:    │  │  │  │  │  │  │  │  │  │  ...   (more pages)
```

- **`Arc<Semaphore>` with `max_workers` permits** — every fetch acquires a permit before connecting; releases on Drop.
- **Level-by-level BFS** — collect all level-N URLs, fetch them concurrently (bounded by the semaphore), extract links, deduplicate, recurse into level N+1.
- **`HashSet<String> visited`** — never fetch the same URL twice.

#### Crawl loop

```rust
use std::sync::Arc;
use std::collections::HashSet;
use tokio::sync::Semaphore;

pub async fn crawl(port: u16, max_workers: usize, max_level: usize) -> CrawlStats {
    let semaphore = Arc::new(Semaphore::new(max_workers));
    let mut visited: HashSet<String> = HashSet::new();
    let mut fetched_urls: Vec<String> = Vec::new();
    let mut discovered_links: HashSet<String> = HashSet::new();

    let mut current_frontier: Vec<String> = vec!["/hub".to_string()];
    visited.insert("/hub".to_string());

    for level in 0..=max_level {
        if current_frontier.is_empty() { break; }

        // Spawn bounded-concurrent fetch tasks for all URLs at this level
        let mut handles = Vec::new();
        for url in current_frontier.drain(..) {
            let sem = semaphore.clone();                    // *** clone PER spawn ***
            handles.push(tokio::spawn(async move {
                let _permit = sem.acquire().await.unwrap(); // RAII: drop returns permit
                let result = fetch_page(port, &url).await;
                (url, result)
            }));
        }

        // Collect results and build the next level's frontier
        let mut next_frontier: Vec<String> = Vec::new();
        for handle in handles {
            let (url, status) = handle.await.unwrap();
            fetched_urls.push(url);
            if let FetchStatus::Ok { body } = status {
                for link in extract_links(&body) {
                    discovered_links.insert(link.clone());
                    if level < max_level && visited.insert(link.clone()) {
                        next_frontier.push(link);
                    }
                }
            }
        }
        current_frontier = next_frontier;
    }

    CrawlStats { pages_fetched: fetched_urls.len(),
                 unique_discovered: discovered_links.len(),
                 fetched_urls }
}
```

**Why a semaphore, not a worker pool.** A worker pool fixes thread count; here we have async tasks on a single multi-threaded runtime. `Arc<Semaphore>(max_workers)` caps **in-flight fetches** — every task holds a permit while it has an open TCP connection. Tokio worker threads are independent (tuning them doesn't affect logical concurrency).

**Why level-by-level BFS, not a global priority queue.** The shortest-path level guarantee comes from "fetch everything at level N, then move to level N+1." A continuous worker pool with a shared queue would interleave levels and break the level invariant.

**Why permits are RAII-only (no `forget()`).** Permits are a resource pool: every `acquire` is matched by a `Drop`. Even on task panic, Drop runs on unwind; the permit returns. **Pool, not gate** — see I5.

**Patterns reused.** I5 (tokio Semaphore as resource pool — RAII drop refunds); I13 (clone-before-move in spawn loops); §3.3 barrier conceptually (each level boundary is a join-all).

**Test pattern (TCP proxy with atomic peak counter).** A3's test suite proves the bound is honored:

```rust
// Peak-concurrency proxy — uses an AtomicUsize CAS loop to track max-ever-seen
let current = active.fetch_add(1, Ordering::SeqCst) + 1;
loop {
    let old_peak = peak.load(Ordering::SeqCst);
    if current <= old_peak { break; }
    if peak.compare_exchange(old_peak, current, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
        break;
    }
}
```

This is the canonical "track maximum-ever value with atomics" pattern — fetch_add + CAS loop on a peak. Composes I4 (CAS loop).

**Common mistakes.**
- Spawning all URLs without the semaphore — observed peak concurrency = total URLs at that level, blowing past `max_workers`.
- Acquiring the semaphore OUTSIDE `tokio::spawn` — every spawn waits its turn at the call site, serializing the spawn loop. The semaphore must be acquired INSIDE the spawned task so all tasks can be scheduled, and only `max_workers` of them actively execute.
- `_permit` as `let _ = sem.acquire().await.unwrap();` — `let _` drops the permit immediately. Must be `let _permit = ...;` (named binding) so the permit lives for the task's lifetime.
- Continuous worker pool with shared queue — breaks level-by-level BFS; pages from level N+1 may be fetched before level N is done.
- Forgetting to add `tokio = { version = "1", features = ["full"] }` to `Cargo.toml`.

---

## 5. Pitfalls catalogue

### Syntax / grammar

1. **`.await()` with parens.** Postfix syntax, not method. `expr.await`.
2. **`acquire().await.forget()` no `.unwrap()`.** `Semaphore::acquire` returns `Result`. Always `.unwrap()`.
3. **`if cond { Err(item) }` then fall-through.** `if` with no `else` evaluates to `()`. Either `return Err(item);` or use `else`.
4. **`Ok()` vs `Ok(())`.** Unit `()` is still a value; `Result<(), T>` `Ok` arm is `Ok(())`.
5. **`std::move(item)`.** Rust moves implicitly.
6. **Missing `;` between statements.** Last expression in block has NO `;`; statements before it need `;`.
7. **`.empty()` on `VecDeque`.** Naming convention: `is_empty`, `is_some`, `is_none`.
8. **`for i in 0...N`.** Removed legacy. Use `0..N` or `0..=N`.
9. **C-style parens around `if` condition.** Idiomatic Rust: `if cond { ... }`.
10. **Stray `;` inside struct literal.** Literals take `field: value` separated by `,`. No statements.
11. **Field naming with trailing `_` (C++ convention).** Rust uses `pub` for visibility, not name mangling.

### Mutex / lock guards

12. **`lock()` without `let mut` binding.** Without binding, guard drops at end of statement → critical section runs UNLOCKED. **Most consequential Rust concurrency mistake.**
13. **`let _ = m.lock();`** Triggers `let_underscore_lock` lint — drops guard immediately. Use `let _g = ...;` (named binding, leading `_`).
14. **`m.lock();`** (statement, no binding). Same problem; guard dies at `;`.
15. **`drop(guard)` to release before `.await`.** Doesn't satisfy Send analysis (lexical, not dataflow). Use block-scope.
16. **Holding `std::sync::Mutex` across `.await`.** `MutexGuard` is `!Send` → future is `!Send` → `tokio::spawn` rejects.
17. **Holding `std::sync::Mutex` reentrantly.** Not reentrant — second `lock()` deadlocks. `wait_while` consumes the guard you already hold.
18. **`*` on the binding instead of the use.** `let mut *g = ...` is a syntax error. `let mut g = ...; *g += 1;`.

### Smart pointers / auto-deref

19. **Operators don't auto-deref through guards/Box/Arc.** `guard == X`, `guard += 1` need `*guard`. Methods do auto-deref.
20. **Cloning `Arc` without binding (silent drop).** `Arc::clone(&x)` → bind to a name → use that name in the closure.

### Predicate / wait API

21. **Polarity inversion when porting from C++.** C++ `wait(lock, pred)` waits *until* true; Rust `wait_while(g, pred)` waits *while* true.
22. **Closure parameter `gen` shadows captured `gen`.** Pick `s` for guard params.
23. **Implicit move of guard into `wait_while` — use-after-move trap.** Shadow the binding with the new guard.
24. **`wait_while(g, pred)` without `.unwrap()`.** Returns `LockResult<MutexGuard>`.
25. **`shared.draining = True` (capitalized boolean).** Rust booleans are `true`/`false`, lowercase.
26. **`notify_one()` when many parked waiters could pass the predicate.** Silent lost wakeup. When unsure, `notify_all`.

### Atomics

27. **`fetch_add(...)` then read `rc` again separately.** Compound-op race. Use **return value** of `fetch_add`.
28. **`if self.rc == 1`.** Can't compare `AtomicI32` directly to `i32`.
29. **`AtomicBool::load()` (no Ordering).** Rust requires explicit `Ordering`; default `SeqCst`.

### Tokio Semaphore

30. **`.release()` on tokio Semaphore.** Doesn't exist. Permits return on Drop.
31. **Forgot `.forget()` on a "consumed" permit.** Permit RAII-returns at scope end. For barriers/gates, `.forget()` is mandatory.
32. **Forgot `add_permits(1)` after `.forget()`.** Permit is gone permanently; semaphore can't recover.

### `tokio::sync::Mutex` / async

33. **`tokio::sync::Mutex::lock()` `.unwrap()` instead of `.await`.** Returns Future, not Result.
34. **`std::sync::Mutex` for a CS that crosses `.await`.** Use `tokio::sync::Mutex`.
35. **`std::thread::sleep()` inside `async fn`.** Blocks the WORKER THREAD. Use `tokio::time::sleep(d).await`.
36. **`std::sync::Barrier::wait()` inside `async fn`.** Same trap. Use `tokio::sync::Barrier`.

### Channels

37. **`Receiver` is NOT `Clone`.** Multi-consumer needs `Arc<Mutex<Receiver<T>>>`.
38. **Forgetting `drop(tx)` in main.** `mpsc` channels close when ALL senders drop. Without explicit drop, consumers hang.
39. **Holding the `Mutex<Receiver>` lock across the blocking `recv()`.** Serialises waiting on top of dequeue. Tight-scope: `let item = { let rx = lock.lock().unwrap(); rx.recv() };`.
40. **`tx.send(value).unwrap()` panics on cancelled receiver.** For release-style semantics, `let _ = tx.send(value);`.
41. **`broadcast::Lagged(n) => break`.** Permanently leaves the subscription pool. Use `continue`.

### `match` and ownership

42. **`match item` then trying to use `item` inside the arm.** `match` consumes `item`. Inside `Ok(s)`, `item` is gone; only `s` remains.

---

## 6. Build / run / debug

```bash
# Regular build
cargo build --workspace
cargo run -p <problem> --release

# ThreadSanitizer (nightly)
RUSTFLAGS="-Z sanitizer=thread" cargo +nightly run -p <problem> --release

# On recent nightlies, sanitized user code can't link against unsanitized prebuilt std:
RUSTFLAGS="-Z sanitizer=thread" cargo +nightly run -p <problem> --release \
    -Z build-std --target x86_64-unknown-linux-gnu

# Loom — exhaustive interleaving exploration
# Cargo.toml: [target.'cfg(loom)'.dependencies] loom = "0.7"
RUSTFLAGS="--cfg loom" cargo test --test my_test --release

# Miri — UB detection (mostly unsafe code, but catches some races)
cargo +nightly miri run

# Stress loop
for i in {1..1000}; do cargo run -p <problem> --release --quiet || break; done
```

**Rust's compile-time push:** the borrow checker, `Send`/`Sync`, `MutexGuard !Send`, `let_underscore_lock` lint, `unused_must_use` on `Result`/`MutexGuard` — all designed to catch concurrency bugs at compile time.

---

## 7. Decision matrix — which primitive when

| Need | Reach for | Why |
|---|---|---|
| Mutex (sync) | `std::sync::Mutex` | RAII guard, faster than tokio. |
| Mutex (held across `.await`) | `tokio::sync::Mutex` | Async-aware. |
| Reader-writer | `std::sync::RwLock` (sync); `tokio::sync::RwLock` (async) | Built-in. |
| Wait-on-predicate (sync) | `Mutex + Condvar + wait_while(g, pred)` | Polarity OPPOSITE C++. |
| Wait-on-predicate (async) | `tokio::sync::Mutex + Notify` (manual loop) | No predicate overload; `Notify` is the wakeup primitive. |
| Counting semaphore | `tokio::sync::Semaphore` | FIFO wakeup. |
| Resource pool (RAII refund) | `tokio::sync::Semaphore` (no `forget`) | Permits auto-return on Drop. |
| Gate / barrier (consume on use) | `tokio::sync::Semaphore` + `forget()` | Tickets consumed, never refunded. |
| Phase synchronization | `tokio::sync::Barrier` (async); `Mutex + Condvar + generation` (sync) | Built-in or hand-rolled. |
| Many-producer, single-consumer (async) | `tokio::sync::mpsc` | Public mailbox. |
| Many-producer, single-consumer (sync) | `std::sync::mpsc` (with `Arc<Mutex<Receiver>>` for multi-consumer) | Single-consumer by design. |
| Addressed single-shot signal (async) | `tokio::sync::oneshot` | Exactly one message. |
| Reusable broadcast wakeup (async) | `tokio::sync::Notify` | Has memory (deposit-permit). |
| Pubsub fan-out (every event) | `tokio::sync::broadcast` | Ring buffer; `Lagged(n)` recoverable. |
| Latest-value broadcast | `tokio::sync::watch` | Receiver IS Clone; lossy by design. |

---
