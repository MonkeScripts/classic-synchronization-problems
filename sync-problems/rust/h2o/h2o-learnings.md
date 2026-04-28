# H2O (Rust + tokio): Mistakes & Learnings

A record of implementing the water-factory problem in Rust with tokio, covering both the **semaphore + barrier** strategy (mirrors the lecture's WaterFactory3) and the **daemon-task** strategy (mirrors the Go daemon).

Companion to `../barbershop/barbershop-learnings.md` — the barbershop notes cover lower-level tokio primitives (`Semaphore` permits, `MutexGuard` deref, `AtomicBool` orderings) that show up here too. This doc focuses on the H2O-specific shapes.

---

## The headline observation

The same problem admits three idiomatic Rust strategies, each with a different center of gravity:

1. **Semaphore + Barrier** — type-cap semaphores limit how many of each kind reach the bonding region; a barrier gates the *start* of bonding so all 3 atoms enter `bond()` together. Compact and stateless. Lives in `src/main.rs`.
2. **Daemon task** — a long-running tokio task orchestrates each molecule. Atoms submit a Request and wait for a "go" signal. The protocol logic lives in one place. Lives in `src/bin/daemon.rs`.
3. **Leader election** (oxygen as leader) — same shape as the daemon, but the leader is *one of the atoms* rather than a separate task. No long-running goroutine, no leak. (Not implemented in this tree, but see `go/h2o_leader/`.)

The hydrogen and oxygen public API is the same in all three (`hydrogen(id).await`, `oxygen(id).await`). What changes is the implementation strategy.

---

## Strategy 1: Semaphore + Barrier (`src/main.rs`)

The protocol:
```rust
async fn hydrogen(&self, id: usize) {
    self.hydrogen_sem.acquire().await.unwrap().forget();   // at most 2 H past this line
    self.barrier.wait().await;                             // wait until 2 H + 1 O are here
    bond_h(id).await;
    self.hydrogen_sem.add_permits(1);                      // let the next H in
}
```

Each acquirer holds the semaphore permit *across the barrier and the bond* — that's the whole point. If you released the permit before bonding, a second H atom could rush past the cap before the barrier had finished forming the molecule, and you'd get more than 2 H in the bonding region.

The `acquire().await.unwrap().forget()` + `add_permits(1)` shape is the classical-Dijkstra pattern from the barbershop notes. Without `.forget()`, the permit RAII-returns to the semaphore at end-of-scope and the cap stops working.

### Mistakes I hit

#### a. `std::sync::Barrier` instead of `tokio::sync::Barrier`

```rust
use std::sync::Barrier;             // ❌ stdlib (sync) barrier
self.barrier.wait().await;          // compile error: not a future
```

The compiler complains that `BarrierWaitResult` is not a future and helpfully suggests "remove the `.await`" — which would compile, but `std::sync::Barrier::wait()` then **blocks the tokio worker thread**. Same trap as `std::thread::sleep` in the barbershop session: a sync blocking call inside an async fn is technically legal but ruins the runtime.

```rust
use tokio::sync::{Semaphore, Barrier};   // ✅
self.barrier.wait().await;
```

> **Rule:** inside an `async fn`, the `tokio::sync::*` and `tokio::time::*` versions of primitives are the right ones. Reach for `std::sync::*` only when the critical section is short and contains no `.await` — see the barbershop notes for the `std::sync::Mutex`-vs-`tokio::sync::Mutex` discussion.

#### b. Stray `;` inside a struct literal

```rust
Self {
    oxygen_sem: Semaphore::new(1),
    hydrogen_sem: Semaphore::new(2),
    barrier: Barrier::new(3);              // ❌ semicolon
}
```

You're inside a `Self { ... }` initializer, which only takes `field: value` pairs separated by `,` — not a function body. Easy slip when you've been writing a lot of statements.

```rust
Self {
    oxygen_sem: Semaphore::new(1),
    hydrogen_sem: Semaphore::new(2),
    barrier: Barrier::new(3),              // ✅ comma
}
```

#### c. Missing `self.` for field access (recurring)

Same Rust-no-implicit-`this->` rule as in the barbershop session. Inside a method, `barrier.wait()` doesn't resolve — you have to write `self.barrier.wait()`. The compiler usually catches this with a helpful "did you mean `self.barrier`?" suggestion.

#### d. Missing `.await` on `barrier.wait()`

`tokio::sync::Barrier::wait()` returns a future. Without `.await`, the future is created and immediately dropped — the barrier never engages and atoms blow past it. The compiler doesn't catch this directly because dropping a future is legal Rust; you only notice when the harness reports `max_total_in_bond != 3`.

> **Rule:** any tokio primitive whose docs mention "blocks until …" returns a future. Always `.await` it. If you're unsure, hover the type — `BarrierWaitResult`, `SemaphorePermit`, `Permit`, etc. all come back from futures.

---

## Strategy 2: Daemon task (`src/bin/daemon.rs`)

The protocol owns two layers of channels:

1. **`mpsc::Sender<Request>` (one per atom type)** — public mailbox. Many atoms send in; the daemon drains.
2. **`oneshot` pairs inside each `Request`** — private one-shot signals. Each Request carries a `go: oneshot::Sender<()>` (daemon→atom) and a `done: oneshot::Receiver<()>` (atom→daemon, with the matching halves kept by the atom).

```rust
struct Request {
    go: oneshot::Sender<()>,
    done: oneshot::Receiver<()>,
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
        h1.go.send(()).unwrap();
        h2.go.send(()).unwrap();
        o.go.send(()).unwrap();
        h1.done.await.unwrap();
        h2.done.await.unwrap();
        o.done.await.unwrap();
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
    // oxygen is symmetric
}
```

### Why TWO oneshots per atom, not one (vs Go's single channel)

Go reuses one bidirectional channel for both "go" and "done" because `chan struct{}` is a *reusable* primitive — every send pairs with a receive on the same wire. Rust's `tokio::sync::oneshot` is **single-shot** — once the value goes through, the channel is dead. So you need two: one for daemon→atom, one for atom→daemon.

Other primitives that *would* allow one channel:
- `tokio::sync::Notify` — repeatable wakeup signal, but `notify_one()` doesn't carry a value (which is fine here since `()` is the signal).
- `tokio::sync::watch` — for state broadcasts.
- An mpsc with capacity 1 — overkill for a two-message conversation.

For per-atom addressed signals, two oneshots is the cleanest shape.

### Why oneshot, not a second mpsc

The done signal is **addressed to one specific atom** (the daemon's "the next loop iteration must wait for THESE three atoms"), not "anyone listening." `oneshot` is the right primitive for "exactly one message to one receiver." An mpsc would require routing logic to figure out which atom's done signal you're getting.

### Mistakes I hit

#### a. `let` statements inside a struct literal

```rust
Self {
    let (h_tx, h_rx) = mpsc::channel(N_HYDROGEN);    // ❌ syntax error
    ...
}
```

Struct literals are field bindings, not function bodies. Statements (`let`, `tokio::spawn`) have to come before the `Self { ... }`:

```rust
let (h_tx, h_rx) = mpsc::channel(N_HYDROGEN);
let (o_tx, o_rx) = mpsc::channel(N_OXYGEN);
tokio::spawn(manager_loop(h_rx, o_rx));
Self { h_tx, o_tx }
```

Also note **field-init shorthand**: when the field name matches the variable name, `Self { h_tx: h_tx }` collapses to `Self { h_tx }`. Idiomatic and avoids repetition.

#### b. `&mut h_rx: mpsc::Receiver<Request>` parameter

```rust
async fn manager_loop(&self, &mut h_rx: mpsc::Receiver<Request>, ...) {  // ❌
```

Three things wrong:
- **`&self`** — the daemon doesn't need any reference to the factory; it only needs the receivers. Drop it.
- **`&mut h_rx: ...`** — that's not a parameter mode; it's an attempt to *pattern-match* a reference. To take ownership of a value and modify it, write `mut h_rx: T`. The `mut` here means "this binding is mutable" (calling `recv` requires `&mut self` on the receiver, which auto-borrows from the binding).
- **It shouldn't be a method.** `tokio::spawn` needs a future. You want to *move* the receivers into the task, not borrow them from `&self`. Standalone `async fn` is the right shape.

```rust
async fn manager_loop(
    mut h_rx: mpsc::Receiver<Request>,
    mut o_rx: mpsc::Receiver<Request>,
) { ... }
```

> **Rule:** a long-running task that owns mutable state is almost always a free `async fn`, not a method. Methods borrow `self`; `tokio::spawn` wants ownership. Move the state in via parameters.

#### c. Calling `.send()` on a Receiver

```rust
h1.done.send(()).unwrap();   // ❌ done is a Receiver
```

`Request.done: oneshot::Receiver<()>`. Receivers don't have `.send` — they have `.await`. The atom owns the matching `done_tx` (Sender) and is the one that calls `send`. The daemon **awaits**:

```rust
h1.done.await.unwrap();      // daemon blocks until atom signals done
```

The asymmetry mirrors the protocol: daemon→atom uses `go.send` + `go_rx.await`; atom→daemon uses `done_tx.send` + `done.await`. Each oneshot has one Sender side and one Receiver side; pick the right method for the half you hold.

### Why `unwrap()` on the sends?

Both `oneshot::Sender::send(value)` and `mpsc::Sender::send(value).await` return a `Result`:

- **`oneshot::Sender::send(value) -> Result<(), T>`** — returns `Err(value)` (the value back!) if the corresponding Receiver was dropped. Nobody's listening; the message can't be delivered.
- **`mpsc::Sender::send(value).await -> Result<(), SendError<T>>`** — same idea, errors when all Receivers are dropped.

In our setup the receiver is *guaranteed* to exist (the atom is parked on `go_rx.await`; the daemon is in its loop), so the error path can't happen. `unwrap()` documents "this can't fail in our setup." Alternatives:

```rust
let _ = h1.go.send(());                                 // ignore quietly
h1.go.send(()).expect("atom dropped its go receiver");  // panic with message
```

For teaching code, `unwrap()` is the right level of "I assert this won't fail." For shutdown-aware production code, you'd handle the error properly.

### Note on `tokio::spawn` inside `new()`

`tokio::spawn` requires being called from within an active tokio runtime. In our setup `main` is `#[tokio::main]`, so any sync function called from it (including `WaterFactory::new()`) inherits the runtime context. But this means **you can't construct a WaterFactory from a non-async context** — the spawn would panic.

If you needed to support that, the workaround is to defer the spawn until the first method call (lazy init), or expose an explicit `start(handle: &Handle)` method that the caller invokes once they're inside a runtime.

---

## Cross-cutting Rust + tokio muscle memory

Consolidating the rules from this session and the barbershop one into a single cheat sheet:

### Tokio primitives

1. **Tokio `Semaphore` permits are RAII.** To use it as a textbook P/V semaphore: `acquire().await.unwrap().forget()` for P, `add_permits(1)` for V. Forgetting `.forget()` silently breaks the cap.
2. **Inside `async fn`, prefer `tokio::sync::*` and `tokio::time::*`** for any blocking primitive (`Mutex`, `Barrier`, `sleep`, `Notify`). The `std::sync::*` versions block the worker thread.
3. **`std::sync::Mutex` is fine inside async** *if* the critical section never spans an `.await`. The tokio docs explicitly recommend it for short sync-only critical sections.
4. **Atomic loads/stores require an explicit `Ordering`.** No defaults. `Ordering::SeqCst` is the right starting point.

### Channel patterns

5. **`mpsc` for many-to-one streams of requests.** The "public mailbox."
6. **`oneshot` for addressed single-fire signals.** "Exactly one message to one specific receiver."
7. **Two oneshots when you need bidirectional one-shot signaling.** Rust oneshots are single-fire; you can't reuse one channel for go-then-done the way Go's `chan struct{}` does.
8. **`Notify` / `watch` / cap-1 mpsc** when you need a reusable signal between two specific tasks.
9. **`tokio::spawn` requires being inside a runtime.** Calling it from `#[tokio::main]`-rooted code is fine; from a sync constructor it's fine *if the caller is in a runtime*. Otherwise it panics.

### Borrow / ownership

10. **Long-running tasks own their state, not borrow it.** Methods on `&self` can't be `tokio::spawn`-ed and hold mutable state across `.await`. Make the daemon a free `async fn` and *move* the state in.
11. **`MutexGuard<T>` auto-derefs for methods/fields, not operators.** `*guard == X` and `*guard += 1` need explicit deref; `guard.method()` doesn't.
12. **`*` belongs at the use site, not the binding.** `let mut g = mutex.lock().unwrap();` then `*g += 1;`. Never `let mut *g = ...`.
13. **`mut` in `mut x: T` means "mutable binding"; `&mut x: T` is a pattern-match attempt and almost always wrong as a parameter mode.**

### Rust syntax that surprised me

14. **Struct literal `T { ... }` only takes `field: value` pairs.** No `let`, no statements, no `;`. Statements go *before* the literal.
15. **Field-init shorthand:** `Self { x }` collapses `Self { x: x }` when names match. Use it.
16. **Use `self.field`, never bare `field`, inside methods.** Rust has no implicit `this->`.
17. **Field naming: no trailing underscore.** Rust uses `pub` for visibility, not name mangling.
18. **`if expr { ... }` — no parens around the condition.** Idiomatic Rust.

### Result/Option discipline

19. **`oneshot::Sender::send` returns `Result<(), T>`.** Returns `Err(value)` (the value back!) if the Receiver was dropped. `unwrap()` is fine when the protocol guarantees a receiver exists; `expect("…")` for clearer messages; `let _ = …` to ignore.
20. **`AtomicBool::load` / `.store` etc. need an `Ordering`.** Default `SeqCst` until you have a reason to weaken.

### Strategy choice

21. **Semaphore + Barrier:** simplest for problems where the constraint is "max-N of each type in the critical section, all rendezvous before starting." No long-running tasks, no leak.
22. **Daemon task:** wins when the protocol logic is intricate enough that you want it in one place. Costs an allocated future per molecule and a goroutine-style leak unless you wire shutdown.
23. **Leader election:** like the daemon but no separate task — one of the participants does double duty. Best for asymmetric problems (here: only one O per molecule, so make the O the leader).
