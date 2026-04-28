# Barbershop (Rust): Mistakes & Learnings

A record of porting the barbershop problem to Rust using `tokio::sync::Semaphore` from async tasks. Companion to `cpp/barbershop/barbershop-learnings.md` and `go/barbershop/barbershop-learnings.md`.

---

## The headline observation

The C++ version taught me **the protocol** (two handshakes around a haircut, four semaphores, one counter mutex). The Go version taught me **how to express it with channels**. The Rust + tokio version forced me to learn two language-specific things that have nothing to do with the protocol itself:

1. **Tokio `Semaphore` permits are RAII — they auto-return on `Drop`.** To use the semaphore as a textbook Dijkstra P/V semaphore (where signals must "stick" until a future P consumes them), every `acquire().await` has to be followed by `.forget()` on the permit. Otherwise the permit pops straight back into the semaphore at end-of-scope and the signal you "consumed" gets re-released.

2. **`MutexGuard<T>` auto-derefs for method calls and field access, but not for operators.** `guard.push()` and `guard.len()` work without `*`. `*guard == X`, `*guard += 1`, and `*guard < CAPACITY` need explicit deref. I'd never noticed this in earlier exercises because I'd only called methods on guards. Reaching for arithmetic on a primitive *through* a guard is the first time the deref-coercion gap surfaces.

Both are language quirks, not protocol-level insights. But each one silently produced a different bug shape, so they're worth naming.

---

## The journey at a glance

| # | Mistake | Fix | Category |
|---|---|---|---|
| 1 | Customer never `V`'d `customer_sem` — barber blocked forever on first customer's arrival | Add `self.customer_sem.add_permits(1);` after the count update | lost wakeup |
| 2 | `customers.lock()` instead of `self.customers.lock()` | `self.` prefix; field access in methods is not implicit in Rust | Rust grammar |
| 3 | `num_customers == CHAIRS` and `num_customers += 1` on a `MutexGuard<usize>` | `*num_customers == CHAIRS` and `*num_customers += 1` — operators don't auto-deref | deref coercion |
| 4 | `let mut *num_customers = self.customers.lock().unwrap();` | `let mut num_customers = ...;` — `*` belongs at the use sites, not the binding | deref placement |
| 5 | `thread::sleep(...)` inside an `async fn` | `tokio::time::sleep(...).await` — sync sleep blocks the worker thread, not the task | async/sync mix |
| 6 | Barber loop only checked `stop` at the top of `while`, not after `customer_sem.acquire()` | Insert `if self.stop.load(Ordering::SeqCst) { break; }` between the acquire and the V | shutdown protocol |
| 7 | `self.stop_.load()` (C++ trailing-underscore convention) and bare `.load()` (no `Ordering`) | `self.stop.load(Ordering::SeqCst)` — Rust fields don't take `_`, atomics require an ordering | C++isms |
| 8 | Default jitter (0..30ms arrival, 2ms cut) → `Balked=0` every run, balk path never exercised | Crank arrivals up / cuts longer to actually fill the shop | testing coverage |

---

## Mistake 1: forgetting to V `customer_sem` (lost wakeup)

### What I wrote
```rust
pub async fn customer(&self, _id: i32) -> bool {
    {
        let mut num_customers = self.customers.lock().unwrap();
        if *num_customers == CHAIRS { return false; }
        *num_customers += 1;
    }
    // ❌ no `self.customer_sem.add_permits(1);` here
    self.barber_sem.acquire().await.unwrap().forget();
    tokio::time::sleep(Duration::from_millis(2)).await;
    self.barber_done.acquire().await.unwrap().forget();
    self.customer_done.add_permits(1);
    // ...
}
```

### What's wrong
The customer atomically reserves a chair, then immediately waits on `barber_sem` for the barber to wave them in — but it never told the barber it was here. The barber is parked on `self.customer_sem.acquire().await`, with no permits, no signal. Both sides are blocked: the customer on `barber_sem`, the barber on `customer_sem`. Classic lost-wakeup deadlock.

### The fix
```rust
self.customer_sem.add_permits(1);                  // ← V customer_sem
self.barber_sem.acquire().await.unwrap().forget(); // P barber_sem
```

### The pairing rule (same as C++ and Go versions)
Every semaphore has **one V'er and one P'er**. The C++ table re-stated for tokio:

| Semaphore | Customer | Barber |
|---|---|---|
| `customer_sem` | `add_permits(1)` | `acquire().forget()` |
| `barber_sem` | `acquire().forget()` | `add_permits(1)` |
| `customer_done` | `add_permits(1)` | `acquire().forget()` |
| `barber_done` | `acquire().forget()` | `add_permits(1)` |

If a row ever has the same op on both sides, the semaphore isn't synchronizing anything.

---

## Mistake 2: `customers.lock()` instead of `self.customers.lock()`

### What I wrote
```rust
pub async fn customer(&self, _id: i32) -> bool {
    {
        let mut num_customers = customers.lock().unwrap();   // ❌ unresolved
```

### What the compiler said
```
error[E0425]: cannot find value `customers` in this scope
help: you might have meant to use the available field
   |   let mut num_customers = self.customers.lock().unwrap();
```

### What's wrong
Coming from C++ I expected member fields to be implicitly in scope inside methods (where C++ has the implicit `this->`). Rust does not — `&self` is a normal parameter and field access requires `self.`. Method bodies don't open the struct's namespace.

### The lesson
Rust is more like Go's `func (bs *Barbershop) Customer()` than C++ in this respect: the receiver is always written explicitly. Saves you from the "is this `customers` a field, a local, or a closure capture?" ambiguity that C++'s implicit `this->` introduces.

---

## Mistake 3: operators don't auto-deref through `MutexGuard`

### What I wrote
```rust
let mut num_customers = self.customers.lock().unwrap();   // MutexGuard<usize>
if num_customers == CHAIRS { return false; }              // ❌
num_customers += 1;                                       // ❌
```

### What's wrong
`std::sync::MutexGuard<T>` is a smart pointer that implements `Deref<Target = T>` and `DerefMut`. Rust's auto-deref only fires for **method calls and field access**. Operators (`==`, `+=`, `<`, etc.) trait-resolve on the **outer type**, not the inner one. `PartialEq<usize>` is implemented for `usize`, not for `MutexGuard<usize>`. `AddAssign` likewise.

So:
- `guard.method()` → auto-deref applies, the method is called on `usize`. ✅
- `guard == X` → trait lookup on `MutexGuard<usize>`. ❌

### The fix
Explicit deref at every operator use:
```rust
if *num_customers == CHAIRS { return false; }
*num_customers += 1;
```

### Why I'd never hit this before
In producer-consumer and readers-writers, the protected state was always a *container* (`VecDeque`, etc.) that I only interacted with via methods (`push_back`, `pop_front`, `len`). Methods auto-deref. The first time you reach for a primitive (`usize`, `bool`, an integer counter) through a guard, you discover the gap.

### Mental model
> Treat `MutexGuard<T>` like `Box<T>`. You can call methods on it transparently, but the moment you want to use the inner `T` as an operand, you write `*guard`.

The same rule applies to `Arc<T>`, `Rc<T>`, `Ref<T>` — every smart pointer Rust ships.

---

## Mistake 4: `let mut *num_customers = ...`

### What I wrote (after learning about deref)
```rust
let mut *num_customers = self.customers.lock().unwrap();   // ❌ syntax error
*num_customers -= 1;
```

### What the compiler said
```
error: expected identifier, found `*`
  --> src/bin/tokio.rs:69:21
69 |   let mut *num_customers = self.customers.lock().unwrap();
   |           ^ expected identifier
```

### What's wrong
Overcorrection. `*` belongs to the **use sites** (where you reach inside the guard), not to the **binding** (where you name the guard itself). The variable is a `MutexGuard`; you bind the guard with a normal name, then deref through it later.

### The fix
```rust
let mut num_customers = self.customers.lock().unwrap();   // bind the guard
*num_customers -= 1;                                       // deref to use the inner usize
```

### Mental model
> `*guard` isn't part of the variable's name — it's an operation applied each time you reach inside. The `let` introduces the guard; the `*` is a separate operation at each use.

This is the same shape as `let p = Box::new(5); *p += 1;`. You don't write `let *p = Box::new(5)`. Same here.

---

## Mistake 5: `thread::sleep` inside `async fn`

### What I wrote
```rust
pub async fn customer(&self, _id: i32) -> bool {
    // ...
    thread::sleep(Duration::from_millis(2));   // ❌
    // ...
}
```

### What's wrong (two layers)
1. **Compile error:** `thread` wasn't imported. `use std::thread;` would fix the symbol resolution.
2. **Runtime semantics, even after fixing imports:** `std::thread::sleep` blocks the entire OS thread, not just the task. In tokio's multi-thread runtime that costs you a worker thread for the duration of the sleep — other tasks scheduled on that worker stop progressing. In a current-thread runtime (`#[tokio::main(flavor = "current_thread")]`) it would deadlock outright: the same thread driving every task gets parked, and no task — including the one that would have signaled the semaphore you're waiting on next — can run.

### The fix
```rust
tokio::time::sleep(Duration::from_millis(2)).await;
```

`tokio::time::sleep` returns a future that yields to the executor, freeing the worker to run other tasks. The current task resumes when the timer fires.

### The general rule
> Inside `async fn`, never call a sync blocking primitive on shared infrastructure. Use the tokio equivalent: `tokio::time::sleep`, `tokio::sync::Mutex` (when held across `.await`), `tokio::fs`, `tokio::net`, etc. For genuinely CPU-bound work, `tokio::task::spawn_blocking`.

There's a related rule about `tokio::sync::Mutex` vs `std::sync::Mutex`:
- `std::sync::Mutex` is fine inside `async fn` **as long as the critical section never spans an `.await`.** The tokio docs explicitly recommend it for short critical sections (cheaper, no async overhead).
- `tokio::sync::Mutex` is required if you hold the lock across `.await` (it integrates with the runtime instead of blocking a worker thread).

In this barbershop, the `customers` counter is incremented/decremented inside a scope that contains no `.await`, so `std::sync::Mutex` is the right choice. Don't reach for `tokio::sync::Mutex` reflexively just because you're in async land.

---

## Mistake 6: shutdown hung — same shape as the C++ version

### What I had
```rust
pub async fn barber(&self) {
    while !self.stop.load(Ordering::SeqCst) {
        self.customer_sem.acquire().await.unwrap().forget();
        // ❌ no stop check here
        self.barber_sem.add_permits(1);
        tokio::time::sleep(Duration::from_millis(2)).await;
        self.barber_done.add_permits(1);
        self.customer_done.acquire().await.unwrap().forget();
    }
}

pub fn shutdown(&self) {
    self.stop.store(true, Ordering::SeqCst);
    self.customer_sem.add_permits(1);   // wake the barber
}
```

### What's wrong
After all 30 customers join, `main` calls `shop.shutdown()`:
1. `stop = true`.
2. `customer_sem.add_permits(1)` — wakes the parked barber.

The barber wakes from `customer_sem.acquire()`, but the loop's `while !self.stop` check happens at the **top of the iteration** (already passed). The barber proceeds straight into the cut sequence:
- V's `barber_sem` (no waiter — permit accumulates).
- Sleeps 2ms.
- V's `barber_done` (no waiter — permit accumulates).
- P's `customer_done` — **blocks forever** because there's no real customer to V it.

`barber_task.await.unwrap()` in `main` never returns. Program hangs. The harness's 5-second timeout caught this when I ran it 20×.

### The fix
Check `stop` **immediately after the acquire**, before any of the cut work:
```rust
loop {
    self.customer_sem.acquire().await.unwrap().forget();
    if self.stop.load(Ordering::SeqCst) { break; }
    self.barber_sem.add_permits(1);
    // ... rest of the cut sequence
}
```

Switching from `while !stop` to `loop` is a code-honesty thing: the only place `stop` ever matters is *after* `acquire`, so put the check there and stop pretending the top-of-loop check is doing anything.

### The pattern (cross-language)
This is **identical to mistake 6 in the C++ version**. Any "long-running worker on a semaphore + shutdown signal" needs:
> A recheck between the wake and the work.

The semaphore primitive can't distinguish "real customer arrived" from "shutdown is using me as a wakeup channel." The disambiguation has to live in the worker, after the wake, before the work commits. Same structural fix as the producer-consumer poison-pill pattern; same shape across C++, Rust, and Go.

---

## Mistake 7: C++isms in Rust syntax

### What I wrote (after the previous fix)
```rust
if (self.stop_.load()) break;
```

### Two C++isms in one line
1. **Field name `stop_` (C++ trailing-underscore convention).** The Rust struct field is plain `stop` (line 39 of `tokio.rs`). Rust doesn't have a private/public naming convention for fields the way C++ does — visibility is controlled by `pub` keywords, not name mangling.
2. **`load()` with no argument.** `std::sync::atomic::AtomicBool::load` requires an `Ordering` parameter. In C++20 the default is `memory_order_seq_cst` if you call `load()` with no args; in Rust there is no default — you have to write the ordering explicitly.

### The fix
```rust
if self.stop.load(Ordering::SeqCst) { break; }
```

(Also dropped the C-style parens around the condition. Idiomatic Rust uses `if expr { ... }`.)

### The lesson
> Porting C++ atomic code to Rust: every `.load()`, `.store()`, `.compare_exchange()` needs an explicit `Ordering`. The compiler won't let you forget — that's the safety property — but the muscle memory is "C++ has a default; Rust does not."

For most purposes in this learning exercise, `Ordering::SeqCst` is the right default. It matches C++'s `memory_order_seq_cst` and is the strongest, simplest-to-reason-about ordering. The weaker orderings (`Acquire`, `Release`, `Relaxed`) are optimizations you'd reach for only after you've measured a contention problem and understand what you're relaxing.

---

## Mistake 8: balk path was never exercised

### What I observed
20 consecutive runs all printed:
```
Served=30 Balked=0 Total=30 (expected 30)
```

The harness assertion `served + balked == TOTAL_CUSTOMERS` passed every time. But notice: **zero balks.** With CHAIRS=3, 0..30ms inter-arrival jitter, and 2ms cuts, the barber drains the queue much faster than customers arrive. The shop never fills.

### Why this matters (same lesson as the C++ version)
The early-return `if *num_customers == CHAIRS { return false; }` and the chair-count balance after `*num_customers -= 1` are *only exercised* when the shop is full. Under the default config they're dead code — I could have written `if *num_customers == 999` and the test would still have passed.

### The fix (for testing, not for code)
Crank up contention. Three knobs:
- Faster arrivals: `rng.gen_range(0..1)` for near-zero gaps.
- Slower cuts: `Duration::from_millis(20)` to make the barber the bottleneck.
- More customers: bump `TOTAL_CUSTOMERS` to ride bursts.

The fix is to the **test config**, not the protocol code — so don't commit it. Run with the stressed values, confirm `Balked > 0`, then revert.

### The bigger lesson (one more time)
> A passing test exercising one half of a protocol isn't a passing test of the protocol. Default-config runs are coverage holes in disguise. If your spec has a balk arm, exercise the balk arm before claiming the protocol works.

---

## Side-by-side: Rust + tokio vs C++ vs Go on this problem

| Concern | C++20 | Go | Rust + tokio |
|---|---|---|---|
| Semaphore primitive | `std::counting_semaphore<>` | `chan struct{}` | `tokio::sync::Semaphore` |
| P (wait) | `.acquire()` | `<-ch` | `.acquire().await.unwrap().forget()` |
| V (signal) | `.release()` | `ch <- struct{}{}` | `.add_permits(1)` |
| Permit lifetime | Counter-based, no RAII | Channel send/recv pair | RAII — drops return permits unless `.forget()` |
| Counter mutex | `std::mutex` | `sync.Mutex` | `std::sync::Mutex` (NOT `tokio::sync::Mutex` since no `.await` is held) |
| Sleep in worker | `std::this_thread::sleep_for(1ms)` | `time.Sleep(2 * time.Millisecond)` | `tokio::time::sleep(...).await` |
| Atomic flag | `std::atomic<bool>` (default seqcst) | atomic.Bool / channel close | `AtomicBool` (Ordering required) |
| Concurrency unit | OS thread | goroutine | tokio task |

The **structure** of the protocol — two handshakes, four signal pairs, one mutex around the counter, one shutdown signal — is identical across all three. What changes is the syntactic surface and the language-specific footguns.

---

## Summary: the Rust-specific muscle memory

After this problem, the patterns I want to internalize:

1. **`tokio::sync::Semaphore` permits are RAII.** To use it as a Dijkstra P/V semaphore, every `acquire().await` must be followed by `.forget()` on the permit, otherwise the permit pops back into the pool at end-of-scope and "consumed" signals get re-released. V is `add_permits(1)`; P is `acquire().await.unwrap().forget()`.

2. **Auto-deref through smart pointers fires for methods and field access, not operators.** `guard.method()` and `guard.field` work; `*guard == X`, `*guard += 1`, `*guard < CAPACITY` need the explicit `*`. Same for `Box`, `Arc`, `Rc`, `Ref`.

3. **`*` belongs at the use site, not the binding.** `let mut g = mutex.lock().unwrap();` then `*g += 1;`. Never `let mut *g = ...`.

4. **Use `self.field`, never bare `field`, inside methods.** Rust has no implicit `this->`.

5. **Inside `async fn`, never call sync blocking primitives.** Use `tokio::time::sleep`, `tokio::sync::*` (when holding across `.await`), `tokio::task::spawn_blocking` for CPU-bound work. Mixing `std::thread::sleep` into async code at best wastes a worker thread; at worst deadlocks a current-thread runtime.

6. **`std::sync::Mutex` is fine inside async if the critical section has no `.await` in it.** Don't reach for `tokio::sync::Mutex` reflexively — the tokio docs prefer std for short, sync-only critical sections.

7. **Atomic loads/stores require an explicit `Ordering`.** No defaults. `Ordering::SeqCst` is the right starting point; reach for weaker orderings only after measuring.

8. **Field naming: no trailing underscore.** Rust controls visibility with `pub`, not name mangling. `self.stop`, not `self.stop_`.

9. **Shutdown signal needs a recheck *after* the wake.** Same rule as C++ and Go. The semaphore wake-up is a coarse signal; the worker has to disambiguate "real work" from "exit now" before committing to a rendezvous that depends on real customers.

10. **Balks need to be observed, not assumed.** A clean `Served=30 Balked=0` run isn't a clean run; it's a run with the balk arm untested. Exercise both arms, or don't trust the green.
