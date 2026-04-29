# FIFO Semaphore (Rust + tokio) — Learnings

Long-form writeup of the conceptual questions and concrete mistakes hit while
implementing `FifoSemaphore` in `sync-problems/rust/fifo_semaphore/src/main.rs`.

The chosen strategy is **Strategy 2 from the file header**: ticket queue
(`next_ticket` / `now_serving`) plus **one `tokio::sync::Notify` per ticket
slot** (`notify_list: [Notify; SLOTS]`). Strategy 3 (oneshot-queue) is the
other clean option — see "What we did not pick" at the end.

---

## Question 1 — "Tokio's `Notify` has no predicate?"

Correct. `tokio::sync::Notify` is **not** a `Condvar`. It has no
`wait_while(predicate)` overload and no atomic "release-mutex-and-wait." The
API is just `notified().await`, `notify_one()`, `notify_waiters()`.

Two consequences if you try to use `Notify` like a `Condvar`:

### 1a. Lost-wakeup hazard with a naive predicate loop

The obvious thing is wrong:
```rust
while self.now_serving.load(Acquire) != my_ticket {
    self.notify.notified().await;   // BUG: race window above
}
```
A `notify_one()` that fires between the load and `notified()` is **lost**.

The corrected pattern registers the future *before* the predicate check:
```rust
loop {
    let fut = self.notify.notified();
    tokio::pin!(fut);
    fut.as_mut().enable();           // "I'm officially waiting" — registers in queue
    if self.now_serving.load(Acquire) == my_ticket { return; }
    fut.await;
}
```
`enable()` is the commit point. Any `notify_one()` after that won't be lost.

### 1b. A single shared `Notify` cannot give FIFO under a predicate

Even though tokio's `Notify` wakes its internal queue FIFO, `release()` calling
`notify_one()` wakes whoever is at the head of the **Notify queue**, which is
not necessarily the waiter whose ticket equals `now_serving`. If ticket 5
happens to be parked ahead of ticket 3 in the Notify queue (e.g. ticket 3
hadn't reached `enable()` yet), ticket 5 wakes, sees `now_serving == 3`, and
has to re-park — burning the wakeup. You'd need to re-broadcast, which is
exactly the "wake the wrong waiter" footgun the file header warns about.

### Why our final design avoids both

We use **one Notify per ticket slot**. So:

- Wakeups are unambiguous — `release()` notifies exactly *the* slot whose
  acquirer is parked there. No stolen wakeups possible.
- Lost-wakeup is handled by `Notify`'s **deposited-permit property**: if
  `notify_one()` fires before `notified().await` registers, the permit sits on
  that slot and is consumed by the next `notified().await` without blocking.
- `notified().await` collapses to one line. No predicate loop, no `enable()`
  dance, no spurious wakeups (tokio Notify guarantees none).

This is the same property the dining-philosophers Tanenbaum implementation
relies on — see comments in `dining_philosophers/src/bin/tokio.rs:282-287`.

---

## Question 2 — "Can I just use `std::sync::Condvar` instead?"

No — `std::sync::Condvar::wait()` parks the **OS thread**, not the task. In a
tokio runtime that means:

- **Multi-thread runtime** (default `#[tokio::main]`): you've pinned one of the
  worker threads. Other tasks scheduled on that worker can't run until the
  Condvar fires. Block enough tasks and you starve the executor (default
  worker count = num CPUs).
- **Current-thread runtime** (`flavor = "current_thread"`): instant deadlock.
  The thread that would have called `release()` to fire the Condvar is the
  same thread you just parked.

A parked OS thread also can't be cancelled by `tokio::time::timeout` or by
dropping a `JoinHandle`.

There is no `tokio::sync::Condvar`. The async-correct alternatives:

| Need                              | Use                                |
|-----------------------------------|------------------------------------|
| Wakeup-with-no-predicate          | `tokio::sync::Notify`              |
| Already-FIFO counting semaphore   | `tokio::sync::Semaphore` (cheating, but allowed in real code) |
| One-shot wake of a specific task  | `tokio::sync::oneshot`             |
| Condvar-shaped predicate-wait     | `tokio::sync::Mutex` + `Notify`    |

**Edge case worth knowing:** `std::sync::Mutex` (without Condvar) is fine
inside an `async fn` *if* you only hold it across non-`await` code. The
blocking is microseconds. Same for `parking_lot::Mutex`. The starter template
exposed `guard: Mutex<()>` for that reason — but we ended up not needing any
mutex (see Question 4).

---

## Question 3 — "With per-ticket `Vec<Notify>`, do I still need the predicate-loop and `enable()`?"

**No.** Both can be deleted. `acquire()` collapses to:
```rust
let my_ticket = self.next_ticket.fetch_add(1, SeqCst);
self.notify_list[my_ticket].notified().await;
```

Why the careful pattern is no longer needed (recap of Question 1):

1. **Wakeup is unambiguous** — only the `release()` that bumps `now_serving`
   past your ticket calls `notify_one()` on *your* slot. No other waiter
   parked there to steal it.
2. **Lost-wakeup is handled by Notify's deposited-permit property** — if
   `release()` fires before you reach `.notified().await`, the permit sits on
   your slot and your next `.notified().await` consumes it immediately.
3. **No spurious wakeups in tokio's Notify** — `notified().await` returning
   *means* someone called `notify_one()` on that slot.
4. **The condition cannot flip back** — once `now_serving` passes your ticket,
   it's monotone. Re-checking would be pointless.

The predicate-loop + `enable()` exists *because* a single shared Notify
creates: (a) the load-then-await window where another waiter's wakeup might
race, and (b) the possibility of being woken when it isn't actually your
turn. Per-ticket slots eliminate both.

### Practical caveats with the `[Notify; SLOTS]` design

- **Sizing.** You need a slot per ticket ever issued. For our test
  (`N_THREADS = 16`, `INITIAL_COUNT = 0`, plus 1 kickoff release), `SLOTS =
  N_THREADS + INITIAL_COUNT + 1` is enough. For an unbounded API you'd need a
  `Mutex<HashMap<u64, Arc<Notify>>>` — at which point Strategy 3
  (oneshot-queue) is much cleaner.
- **Seeding `initial_count`.** See Question 5.

---

## Question 4 — "Why no mutex? I'm scared of concurrent modifications to the list."

The fear is reasonable — but there's nothing here that needs guarding.
Separate the two things "the list" might mean:

### 4a. The array's spine (`[Notify; SLOTS]`)

Never modified after `new()` returns. No push/pop/resize/replace. It's a
fixed-size array of `Notify` objects living inside the `Arc<FifoSemaphore>`.
Concurrent tasks only ever **read** the array to obtain a `&Notify` to a slot
— and reading the same `&self` from many tasks is exactly what `Arc` allows.
Concurrent modification of the list cannot happen by construction.

### 4b. The contents of an individual `Notify`

This *is* concurrent — `release()` calls `notify_one()` on the same slot the
acquirer is calling `notified().await` on. But `Notify` is **internally
synchronized** — that's its whole job. Each `Notify` holds its waiter queue
inside its own mutex + atomic state. `notify_one()` and `notified().await`
both take `&self` (not `&mut self`); the type system is telling you "no
exclusive access required."

Wrapping `Notify` in your own `Mutex` would **double the locking** (your
mutex + Notify's internal mutex) for zero correctness benefit.

### 4c. "But two tasks might hit the same slot at the same time"

That's both fine *and* rare-by-construction:

- Two acquirers cannot get the same ticket: `next_ticket.fetch_add(1, SeqCst)`
  returns a unique value to every caller. That's the entire point of
  `fetch_add` — it's an atomic read-modify-write (`lock xadd` on x86).
- Two releases cannot admit the same ticket: same argument with `now_serving`.
- The only "same slot, two parties" case is `release()` calling `notify_one()`
  on slot `t` while the acquirer with ticket `t` calls `notified().await` on
  the same slot — and that's exactly the case `Notify` is built to handle,
  including the case where the notify lands before the await registers.

### 4d. The atomics do the work the mutex would have done

In a non-atomic version you'd write:
```rust
let mut g = self.guard.lock().await;
let my_ticket = self.next_ticket;
self.next_ticket += 1;
drop(g);
```
`fetch_add(1, SeqCst)` collapses those four lines into one CPU instruction
with the same ordering guarantees. The mutex was never adding correctness;
it was making the read-modify-write atomic. The atomic does that natively.

### 4e. When you *would* need a mutex

If your design held **compound** state that has to move together. Example:
the oneshot-queue (Strategy 3) does
`Mutex<VecDeque<oneshot::Sender<()>>>` because `release()` must `pop_front`
**and** `send` and those two operations have to be observed together.
Here, every shared-mutable field is a single integer with single-step
transitions, so atomics suffice.

---

## Question 5 — "Why pre-deposit `initial_count` permits in `new()`?"

To honor the API contract:
> `new(initial_count)` ... `acquire().await: block until count > 0`

If `initial_count = 3`, the first three `acquire()` calls **must return
without blocking** — those three permits exist before anyone calls
`release()`.

In our design `acquire()` always parks on a slot:
```rust
let my_ticket = self.next_ticket.fetch_add(1, SeqCst);
self.notify_list[my_ticket].notified().await;     // ← always awaits
```

If we built `new(3)` with empty Notify slots, the first acquirer would
ticket into slot 0, hit `notified().await`, and hang. Contract violation.

The preload deposits `initial_count` permits using Notify's deposited-permit
property, and starts `now_serving` at `initial_count` so the first **real**
release lands on the first **truly blocked** waiter:
```rust
let s = Self {
    now_serving: AtomicUsize::new(initial_count),
    next_ticket: AtomicUsize::new(0),
    notify_list: std::array::from_fn(|_| Notify::new()),
};
for i in 0..initial_count {
    s.notify_list[i].notify_one();   // pre-deposit
}
s
```

### Worked example, `initial_count = 2`

```
new(2):       slots [P, P, _, _, _]   now_serving=2  next_ticket=0
acq #1 (t=0): consumes slot 0          → returns immediately
acq #2 (t=1): consumes slot 1          → returns immediately
acq #3 (t=2): awaits slot 2 (empty)    → BLOCKS
acq #4 (t=3): awaits slot 3 (empty)    → BLOCKS
release():    now_serving 2→3, notify slot 2 → acq #3 wakes
release():    now_serving 3→4, notify slot 3 → acq #4 wakes
```

For our test specifically, `INITIAL_COUNT = 0`, so the preload loop runs
zero times. It's a no-op for our harness. It only matters if you ever
construct `FifoSemaphore::new(K)` with `K > 0`.

### Don't double-count `now_serving`

Two ways to do the seed; pick exactly one:
```rust
// Option A — start at 0, let the loop bump it:
now_serving: AtomicUsize::new(0),
for i in 0..initial_count {
    s.notify_list[i].notify_one();
    s.now_serving.fetch_add(1, SeqCst);
}
```
```rust
// Option B — start at initial_count, no bump in loop:
now_serving: AtomicUsize::new(initial_count),
for i in 0..initial_count {
    s.notify_list[i].notify_one();
}
```
Doing both (start at `initial_count` *and* bump in the loop) was a real
mistake during this implementation: `now_serving` ends at `2 *
initial_count`, and the first `K` real releases skip the first `K` blocked
waiters entirely.

---

## Concrete mistakes hit during implementation

In rough order they appeared:

### M1. Holding the mutex across `.await`

```rust
pub async fn acquire(&self) {
    let _guard = self.guard.lock().await;
    let current_ticket = self.next_ticket.fetch_add(1, SeqCst);
    self.notify_list[current_ticket].notified().await;   // <-- still holding _guard
}
```
Deadlock: while task A holds `_guard` and awaits its slot, no other task can
enter `release()` because they block on `guard.lock().await`. Nobody can
ever wake A.

**Rule:** never hold a tokio `Mutex` (or `std::sync::Mutex`) across `.await`
unless you've thought carefully about whether anything that could unblock
the `.await` also needs the mutex.

### M2. `release()` not declared `async` but using `.await`

```rust
pub fn release(&self) {                       // not async
    let _guard = self.guard.lock().await;     // compile error
    ...
}
```
Two fixes possible: drop the mutex (correct fix here) or make `release()`
async. We dropped the mutex.

### M3. Off-by-one in ticket alignment

Setting `next_ticket: AtomicU8::new(1)` while leaving `now_serving =
initial_count = 0`. First acquirer fetched ticket 1 and parked on slot 1.
Kickoff `release()` notified slot 0 — permit deposited where nobody
listens. Acquirer on slot 1 hangs.

**Rule:** `next_ticket` and `now_serving` must be in agreement at startup.
For `initial_count = 0` both start at 0; for `initial_count = K` both can
start at `K` (Option B preload) or both at 0 with the loop bumping
`now_serving` (Option A).

### M4. `initial_count` double-counted in `new()`

Initialized `now_serving` to `initial_count` AND bumped it `initial_count`
times in the preload loop. End state: `now_serving = 2 * initial_count`.
First `K` real releases miss their waiters.

### M5. `AtomicU8` indexed into `[Notify; SLOTS]` without cast

Rust array indices must be `usize`. `AtomicU8::fetch_add` returns `u8`.
Either cast at the call site (`as usize`) or — cleaner — use `AtomicUsize`
end-to-end and skip the casts entirely. We did the latter.

### M6. Duplicate `use` blocks

`use std::sync::atomic::{AtomicUsize, Ordering};` etc. appearing twice.
The compiler rejects duplicate names in the same scope. Not interesting,
just a copy-paste hazard when adding new imports.

### M7. Stray `guard: Mutex::new(())` in struct literal after removing the field

Removed `guard` from the struct definition but left it in `Self { ... }`,
which then references both a non-existent field and an unimported type.
Two errors in one line.

### M8. Slot array sized to `[Notify; N]` where `N` is undefined

Original code referenced an undefined `N`. The constants in the file are
`N_THREADS`, `INITIAL_COUNT`, etc. Solution: introduce
`const SLOTS: usize = N_THREADS + INITIAL_COUNT + 1;`. The `+ 1` covers the
external kickoff `release()` in `main`.

---

## Final working shape — Strategy 2 (`ticket_notify.rs`)

```rust
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;

const N_THREADS: usize = 16;
const INITIAL_COUNT: usize = 0;
const ARRIVAL_SPACING_MS: u64 = 10;
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
        for i in 0..initial_count {
            s.notify_list[i].notify_one();
        }
        s
    }

    pub async fn acquire(&self) {
        let current_ticket = self.next_ticket.fetch_add(1, Ordering::SeqCst);
        self.notify_list[current_ticket].notified().await;
    }

    pub fn release(&self) {
        let finished_ticket = self.now_serving.fetch_add(1, Ordering::SeqCst);
        self.notify_list[finished_ticket].notify_one();
    }
}
```

Validates with 0 FIFO violations across ~20 runs. A more convincing stress
test:
```bash
cd sync-problems/rust
for i in {1..1000}; do
    cargo run -p fifo_semaphore --release --quiet || break
done
```
Re-run under TSan when you want to be paranoid:
```bash
RUSTFLAGS="-Z sanitizer=thread" cargo +nightly run -p fifo_semaphore --release
```

---

## Strategy 3 — `oneshot_queue.rs`

Second-pass implementation that mirrors C++ `FIFOSemaphore5`. State is a
`VecDeque<oneshot::Sender<()>>` plus a counter, both protected by one
`std::sync::Mutex`. FIFO is automatic (VecDeque preserves push order;
`pop_front` always wakes the oldest waiter; oneshot send/recv is
unambiguous — no shared Notify slots, no permit-can-wake-the-wrong-task
hazard).

The remaining questions are about *why the design has the shape it does*,
not whether it's correct.

---

## Question 6 — "Why does the Mutex wrap a `State` struct? It looks weird to nest everything inside one mutex."

This is **data locking** vs **code locking** — a deliberate Rust choice, not
a quirk.

### C/C++/Go style (code locking)

```cpp
std::mutex m;
size_t count;
std::deque<...> waiters;
```
The mutex and the protected data are separate. The programmer must
*remember* to lock `m` before touching `count` or `waiters`. The compiler
provides no help. Forget once → race condition.

### Rust style (data locking)

```rust
struct State {
    count: usize,
    waiters: VecDeque<oneshot::Sender<()>>,
}
state: Mutex<State>,
```
The mutex **owns** the data. There is no syntactic way to access `count`
or `waiters` without first calling `state.lock()` and dereferencing the
`MutexGuard`. The type system makes "forgot to lock" structurally
impossible. The wrapping looks heavy at first glance but it's actively
preventing a class of bugs.

### When you'd split it

You group inside one `Mutex<...>` exactly the fields that share a
**compound invariant** — fields that must be observed together. Here,
`release()` does *either* `pop_front + send` *or* `count += 1`, and that
branch must be atomic w.r.t. `acquire()`. So `count` and `waiters` belong
inside the same mutex.

If a field has no compound relationship with the rest — a standalone
counter, a stats gauge — pull it out as its own `AtomicUsize` (or its own
`Mutex<T>`). Read-only-after-construction fields don't need either.

This is exactly why `ticket_notify.rs` has *no* mutex: each integer is
independent (`fetch_add` is atomic), and the Notify spine is read-only.
There's no compound invariant to protect.

---

## Question 7 — "Why doesn't `drop(state); rx.await;` work? I dropped the lock before the await, isn't that enough?"

**No, when the future will be `tokio::spawn`ed.** This is the most
surprising trap of the whole exercise.

### What happens

```rust
pub async fn acquire(&self) {
    let mut state = self.state.lock().unwrap();
    if state.count > 0 { state.count -= 1; return; }
    let (tx, rx) = oneshot::channel();
    state.waiters.push_back(tx);
    drop(state);          // <-- explicit drop
    rx.await.unwrap();    // <-- compiler complains: future is not Send
}
```
Compile error:
```
the trait `Send` is not implemented for `std::sync::MutexGuard<'_, State>`
note: future is not `Send` as this value is used across an await
   |   let mut state = self.state.lock().unwrap();
   |       --------- has type `std::sync::MutexGuard<'_, State>` which is not `Send`
   |   rx.await;
   |      ^^^^^ await occurs here, with `mut state` maybe used later
note: required by a bound in `tokio::spawn`
```

### Why

When `async fn acquire` is compiled, every local that *might* be alive at
any `.await` point gets baked into the generated future state-machine.
`tokio::spawn` requires the future to be `Send`. The auto-trait analyser
that decides whether the future is `Send` does **not** look at explicit
`drop()` calls — it conservatively assumes `state` is still live across
the await. `MutexGuard` is `!Send`, so the future is `!Send`, so the spawn
fails.

### What does work

**Block scope.** A value that goes out of scope at the end of a `{ ... }`
block *is* recognised by the auto-trait analyser as dead at the next
statement:

```rust
pub async fn acquire(&self) {
    let rx = {
        let mut state = self.state.lock().unwrap();
        if state.count > 0 { state.count -= 1; return; }
        let (tx, rx) = oneshot::channel();
        state.waiters.push_back(tx);
        rx
    };                    // ← state guard is GUARANTEED dead here
    rx.await.unwrap();
}
```

`return` inside the block exits the whole function (not just the block);
the guard drops during stack unwind on the way out. No await ever runs
with a `MutexGuard` alive.

### When `drop()` *is* enough

If the future is *not* `Send`-required — e.g. a `tokio::task::spawn_local`
on a `LocalSet`, or running on the current task — `drop(state)` is
sufficient because the auto-trait check doesn't fire. But the moment you
cross a worker-thread boundary with `tokio::spawn`, the block-scope
pattern is mandatory.

### Why `release()` doesn't have this problem

`release()` is `pub fn` (not async) and never `.await`s. The MutexGuard
lives on the OS thread stack, not inside a future. `drop(state)` works
fine there — and is just hygiene; the guard would drop at end-of-scope
anyway.

---

## Concrete mistakes hit during Strategy 3 implementation

### M9. `acquire()` doesn't early-return on the fast path

Falls through after decrementing, pushes a `tx` no one knows about, then
awaits an `rx` that nobody will ever `send()` to → silent deadlock. Must
`return;` immediately after `state.count -= 1;`.

### M10. `let mut count = state.count;` mutates a copy

`usize` is `Copy`. The local `count -= 1` discards the change at the end
of the function. Always mutate `state.count` directly.

### M11. `self.waiters.push(tx)`

Two errors in one line: `waiters` is on `state`, not `self`; and
`VecDeque` has no `push` (it's `push_back` for FIFO).

### M12. `.await` on `std::sync::Mutex::lock()`

`std::sync::Mutex::lock()` is **synchronous** — it returns
`LockResult<MutexGuard<...>>` immediately, not a future. Drop the
`.await`. (And pair it with `.unwrap()` to handle the poison case.) Same
mistake on `release()` — plus that function isn't even `async`, so
`.await` would have failed there too.

### M13. `pop_front()` returns `Option<T>`, not `T`

You can't call `.send()` on an `Option`. Use `if let Some(tx) =
state.waiters.pop_front() { ... }`. As a bonus, the `if let` binds the
non-empty case structurally, so the empty-branch `count += 1` is
automatic.

### M14. `tx.send(()).unwrap()` panics on cancelled receiver

`oneshot::Sender::send` returns `Err(())` if the `Receiver` was dropped
(e.g. because a `tokio::time::timeout` cancelled the awaiter). For
release semantics we want to silently ignore that case:
```rust
let _ = tx.send(());
```

### M15. `drop(state); rx.await;` — the `Send`-bound trap

See Question 7. Use the `let rx = { ... }; rx.await;` block pattern.

### M16. Missing `mut` on the lock binding

`let state = self.state.lock().unwrap();` only allows reads. Mutating
`state.count` or calling `state.waiters.pop_front()` requires
`let mut state = ...`.

### M17. `VecDeque::empty()` is not a method

It's `is_empty()`. (`Vec::empty()` doesn't exist either — confused with
Python / Java / Go.)

### M18. Closing `}` of `acquire()` accidentally deleted

When restructuring `acquire()` to use a block, the final `}` of the
function body got dropped, so `release()` ended up nested *inside*
`acquire()`. Parser bailed out a hundred lines later with a generic
"unclosed delimiter" error pointing at the wrong brace. Worth a
double-check whenever you restructure a method body.

---

## Final working shape — Strategy 3 (`oneshot_queue.rs`)

```rust
use std::collections::VecDeque;
use std::sync::Mutex;
use tokio::sync::oneshot;

struct State {
    count: usize,
    waiters: VecDeque<oneshot::Sender<()>>,
}

pub struct FifoSemaphore {
    state: Mutex<State>,
}

impl FifoSemaphore {
    pub fn new(initial_count: usize) -> Self {
        Self {
            state: Mutex::new(State {
                count: initial_count,
                waiters: VecDeque::new(),
            }),
        }
    }

    pub async fn acquire(&self) {
        let rx = {
            let mut state = self.state.lock().unwrap();
            if state.count > 0 {
                state.count -= 1;
                return;
            }
            let (tx, rx) = oneshot::channel();
            state.waiters.push_back(tx);
            rx
        };
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

Validates with 0 FIFO violations across 20 runs:
```bash
for i in {1..20}; do
    cargo run -p fifo_semaphore --bin oneshot_queue --release --quiet | tail -1
done
```

---

## Comparing the two strategies

| Aspect                         | `ticket_notify` (Strategy 2)             | `oneshot_queue` (Strategy 3)              |
|--------------------------------|------------------------------------------|-------------------------------------------|
| Coordination primitive         | `[Notify; SLOTS]` + two `AtomicUsize`    | `Mutex<State>` with `VecDeque` + counter  |
| Compound invariant?            | No — each field independent              | Yes — `pop_front` *or* `count += 1`       |
| Mutex needed?                  | No (`fetch_add` suffices)                | Yes (`fetch_add` can't express compound)  |
| Capacity                       | Fixed-size slot array                    | Unbounded VecDeque                        |
| FIFO mechanism                 | Per-ticket Notify, one waiter per slot   | Per-waiter oneshot, queue order           |
| Initial-count seeding          | Pre-deposit permits + bump `now_serving` | Just set `count = initial_count`          |
| `Send`-bound trap applicable?  | No (no Mutex held across await)          | **Yes** — must use block-scope around lock |
| Closest analogue               | Textbook ticket lock                     | C++ `FIFOSemaphore5`                      |
| Production-shape?              | Capacity-bounded; simpler if N is fixed  | More flexible; standard Rust pattern      |

Both pass `for i in {1..20}; do cargo run ...; done` with zero FIFO
violations on the test harness. For paranoia, stress-loop to 1000 runs
and re-run under TSan:
```bash
RUSTFLAGS="-Z sanitizer=thread" cargo +nightly run -p fifo_semaphore --bin oneshot_queue --release
```
