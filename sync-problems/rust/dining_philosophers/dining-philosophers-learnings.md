# Dining Philosophers in Rust: Mistakes & Learnings

Companion to `cpp/dining_philosophers/dining-philosophers-learnings.md` and `go/dining_philosophers/dining-philosophers-learnings.md`. One binary in this crate hosts all five strategies:

- `dining_philosophers/src/bin/tokio.rs` — `eat_naive`, `eat_asymmetric`, `eat_try_backoff`, `eat_footman`, `eat_tanenbaum`. Built on `tokio::sync::{Mutex, Semaphore, Notify}` because `std` has no async-aware mutex and we want to hold guards across `.await`.

The mistakes in this problem clustered around two themes:

1. **Lock-guard lifetime mistakes** — forgetting that a `MutexGuard` is a *value* you must bind with `let mut state = …`, otherwise it drops on the same line and you have no lock.
2. **RAII permit semantics** — understanding that `SemaphorePermit::Drop` is what refills the semaphore, and that `forget()` is the explicit opt-out for one-shot signal idioms.

---

## The journey at a glance

| # | Mistake | What it taught me |
|---|---|---|
| 1 | `first_chopstick = shared.chopsticks[i].lock().unwrap()` | Two errors at once: missing `let`, and `tokio::sync::Mutex::lock()` returns a `Future`, not `Result`. The right shape is `let _g = m.lock().await;` |
| 2 | `let mut first = pid; if pid == N-1 { first = …; }` | `if/else` is an expression in Rust. `let (first, second) = if cond { (a, b) } else { (c, d) };` is both clearer and immutable |
| 3 | `break;` from a retry loop, then trying to use the guards outside | Guards bound *inside* the loop body drop when control leaves the body. `loop` is an expression — use `let (l, r) = loop { … break (l, r); };` to lift them to the outer scope |
| 4 | `shared.chopsticks[right].lock().try_lock()` | Double API call. `.lock()` returns a Future (which has no `try_lock`). For non-blocking, just `.try_lock()` directly on the mutex |
| 5 | `println!("{}", *guard)` for `Mutex<()>` | `()` doesn't implement `Display`. The unit mutex is a marker — there's nothing to print. Print the index/pid instead, or delete the line |
| 6 | `shared.algo_states[pid].lock().await` | `algo_states: Mutex<[u8; N]>` is **one** mutex around an array, not `[Mutex<u8>; N]`. You can't index a `Mutex`. Lock once, get `&mut [u8; N]` |
| 7 | `shared.algo_states.lock().await` (no `let`) | The `MutexGuard` is the binding that holds the lock. Without `let mut state = …`, the guard drops at end-of-statement and the lock releases instantly |
| 8 | `state[pid].store(HUNGRY, Ordering::SeqCst)` inside the algo lock | Conflated the two layers. **Oracle** (`shared.states`) is `AtomicU8`, uses `.store()`. **Algo state** (`shared.algo_states`) is plain `u8` inside a mutex, uses `=` |
| 9 | `shared.notify[pid].notified().await` *inside* the algo lock | Awaiting while holding the lock = nobody can ever signal you. `.notified().await` must happen *after* the lock guard drops |
| 10 | "Should I `.forget()` the footman permit?" | No. Footman is a *resource pool* — RAII drop refills the seat. Only the *gate/signal* idiom (barrier) needs `.forget()` |

---

## Mistake 1: missing `let` + `.unwrap()` on `lock()`

### What I wrote
```rust
async fn eat_asymmetric(shared: &SharedTokio, pid: usize) {
    // …
    first_chopstick = shared.chopsticks[first].lock().unwrap();
    second_choptstick = shared.chopstick[second].lock().unwrap();
    // …
}
```

### What's wrong
Three problems stacked:

1. `first_chopstick = …` is an *assignment* to an undeclared binding. Rust requires `let` to introduce a new binding.
2. `tokio::sync::Mutex::lock()` returns `impl Future<Output = MutexGuard<'_, T>>` — there's no `Result`, so `.unwrap()` is a type error. You await it.
3. (Plus a typo: `chopstick` vs `chopsticks` and `choptstick`.)

### The fix
```rust
let _first_chopstick = shared.chopsticks[first].lock().await;
let _second_chopstick = shared.chopsticks[second].lock().await;
```

### Why is it a `Future`, not a `Result`?
Two reasons it's infallible — unlike `std::sync::Mutex`:

- **No poisoning.** `std::sync::Mutex::lock()` returns `LockResult` because a panic-while-holding marks the lock poisoned. Tokio's mutex doesn't track that.
- **No "would block" branch.** That only shows up on `try_lock()`, which returns `Result<MutexGuard, TryLockError>`.

So the rule of thumb in the dining-philosophers file:

| Call | Returns |
|---|---|
| `mutex.lock().await` | bare `MutexGuard` |
| `mutex.try_lock()` | `Result<MutexGuard, TryLockError>` |
| `sem.acquire().await` | `Result<SemaphorePermit, AcquireError>` (errors only after `close()`, never in our code) |

### The leading underscore
`_first_chopstick`, not `first_chopstick`. The leading `_` tells the compiler "I'm holding this for its `Drop` side-effect, not to read it" — suppresses the `unused_variable` warning. This is the Rust idiom for RAII guards.

---

## Mistake 2: `let mut` + reassign instead of `if/else` as expression

### What I wrote
```rust
let mut first = pid;
let mut second = (pid + 1) % N;
if pid == N - 1 {
    first = (pid + 1) % N;
    second = pid;
}
```

Works, but mutable bindings I'll never reassign again, plus a "set defaults then maybe overwrite" structure that hides the intent.

### The fix
```rust
let (first, second) = if pid == N - 1 {
    ((pid + 1) % N, pid)
} else {
    (pid, (pid + 1) % N)
};
```

### The bigger lesson
`if/else` is an **expression** in Rust — the whole construct evaluates to whichever arm runs. Combined with destructuring `let`, you express "decide once, bind two values" in a single statement. The bindings are *immutable*, which is what they should be — once chosen, you don't reassign.

This is the same family as `loop`-as-expression (Mistake 3) and `match`-as-expression. Rust expresses control flow through values rather than through mutation, and the type system pushes you toward that style by making the alternative awkward.

---

## Mistake 3: `break` from a retry loop loses the guards

### What I tried
```rust
loop {
    let l_guard = shared.chopsticks[left].lock().await;
    match shared.chopsticks[right].try_lock() {
        Ok(r_guard) => break,            // ❌ guards die here
        Err(_) => drop(l_guard),
    }
    // … phase B …
}

// Eat with both guards held — but they already dropped on the way out.
shared.states[pid].store(EATING, Ordering::SeqCst);
```

### What's wrong
`l_guard` and `r_guard` are bound *inside* the loop body. When control leaves the body via `break`, they go out of scope and `Drop` runs — releasing the chopsticks before the eat-section. You hit `EATING` holding nothing, the invariant fires.

### The fix
`loop` is an expression in Rust, and `break <value>` makes the loop *evaluate* to that value. Bind the guards in the **outer** scope:

```rust
let (_l, _r) = loop {
    // Phase A: lock left, try right
    let l_guard = shared.chopsticks[left].lock().await;
    match shared.chopsticks[right].try_lock() {
        Ok(r_guard) => break (l_guard, r_guard),
        Err(_) => drop(l_guard),
    }

    // Phase B: lock right, try left
    let r_guard = shared.chopsticks[right].lock().await;
    match shared.chopsticks[left].try_lock() {
        Ok(l_guard) => break (l_guard, r_guard),
        Err(_) => drop(r_guard),
    }
};

// _l and _r are alive in this scope — eat normally, no duplication.
```

### Three things this shape buys you
1. **`break (a, b)` returns a tuple from the loop**, destructured by `let (_l, _r) = …`. Same idea as `if`-as-expression; `loop` is an expression too.
2. **Guards live in the function scope**, so they survive into the eat-section.
3. **The eat-section appears once.** The TODO template suggested duplicating the five lines into both match arms — this loop-as-expression form avoids that.

### Why explicit `drop(l_guard)` matters in Phase A's `Err` arm
If you fall through to Phase B still holding the left guard, then call `lock().await` on right, you've reproduced the exact deadlock the strategy is supposed to avoid: holding left, blocking on right, while a neighbor holds right and blocks on you. The explicit `drop()` releases left *before* Phase B even starts.

---

## Mistake 4: `lock().try_lock()` — double API call

### What I wrote
```rust
match shared.chopsticks[right].lock().try_lock() {
    Ok(g) => …,
    Err(_) => …,
}
```

### What the compiler said
`Future` (the return of `.lock()`) has no method `try_lock`. The chain doesn't typecheck.

### The fix
```rust
match shared.chopsticks[right].try_lock() {
```

### The bigger lesson
`.lock()` and `.try_lock()` are siblings on `Mutex`, not stacked. Even if `lock().try_lock()` did compile, it'd be wrong: `.lock()` queues you in the wait-list, defeating the whole "non-blocking try" point.

---

## Mistake 5: trying to print `*MutexGuard<()>`

### What I wrote
```rust
println!("Acquired lock: {}", *second_chopstick);
```

### What's wrong
`chopsticks: [Mutex<()>; N]` — the inner type is unit. Dereferencing the guard gives `()`, which has no `Display` impl. Compile error.

### The fix
Either delete the print, or reach for something printable:
```rust
println!("pid={pid} got chopstick {right}");
```

### Why the unit mutex?
The chopsticks aren't carrying data — they're pure mutual-exclusion markers. `Mutex<()>` is the idiomatic "lock with no payload" in Rust. The guard exists *only* for its `Drop` side-effect. There's nothing inside to print, by design.

---

## Mistake 6: indexing a `Mutex<[u8; N]>`

### What I wrote
```rust
let mut state = shared.algo_states[pid].lock().await;
```

### What's wrong
The field declaration is:
```rust
algo_states: Mutex<[u8; N]>,
```

This is **one** mutex wrapping an array. Not `[Mutex<u8>; N]` (an array of N separate mutexes). So `algo_states[pid]` doesn't compile — `Mutex` is not indexable.

### The fix
```rust
let mut state = shared.algo_states.lock().await;
// state is a MutexGuard that derefs to &mut [u8; N]
state[pid] = HUNGRY;
```

### Why one big lock instead of N small ones
The Tanenbaum algorithm needs **atomic decisions across multiple philosophers' states**: when philosopher P finishes, it tests both neighbors and may transition them. If each philosopher's state had its own mutex, you'd need to acquire two of them in some order — and that brings back the deadlock-avoidance problem this strategy was supposed to dodge entirely. One mutex over the whole array makes each test/transition trivially atomic.

---

## Mistake 7: forgetting `let mut state = …`

### What I wrote (paraphrased from the bug-hunting session)
```rust
{
    shared.algo_states.lock().await;     // ❌ guard drops here, lock released immediately
    shared.tanenbaum_test(/* … */);      // not actually under the lock!
}
```

### What's wrong
`shared.algo_states.lock().await` produces a `MutexGuard`, but if you don't bind it, the guard is dropped at the end of the statement (the `;`). The lock releases instantly — before you've done any work under it. Subsequent code in the block looks like it's inside the critical section, but it isn't.

### The fix
```rust
{
    let mut state = shared.algo_states.lock().await;
    state[pid] = HUNGRY;
    shared.tanenbaum_test(&mut state, pid);
}   // <-- guard drops HERE, lock released HERE
```

### Why this is *the* most consequential Rust concurrency mistake
Other languages make you write `lock.acquire(); … lock.release();` and forgetting the release is a forever-held lock. Rust's RAII flips this: forgetting to *bind* the guard is a never-held lock. The lock acquires and releases in one expression, and the rest of the block runs without protection.

The compiler doesn't help you here because dropping a `MutexGuard` immediately is a *valid* program — it just isn't what you meant. There's no warning. The bug surfaces only as a data race or a failed invariant under contention.

The mental rule:
> Every `lock()` / `lock().await` must be on the right-hand side of a `let` (named binding). If you're not binding it, you're not locking.

The leading underscore convention helps: `let _guard = m.lock().await;` makes the intent explicit ("I want this for Drop, not to read"). `let _ = m.lock().await;` does **not** work — `let _` drops the value immediately, same problem. Always a *named* binding.

---

## Mistake 8: oracle vs algo state — atomic API on plain `u8`

### What I wrote
```rust
let mut state = shared.algo_states.lock().await;
state[pid].store(HUNGRY, Ordering::SeqCst);   // ❌
```

### What's wrong
`shared.states[pid]` is `AtomicU8` — uses `.store(value, ordering)`.
`state[pid]` (inside the algo lock) is plain `u8` — uses `=`.

The two layers exist for a reason:

| Layer | Field | Type | API | Read by |
|---|---|---|---|---|
| **Oracle** | `shared.states` | `[AtomicU8; N]` | `.store()`, `.load()` | `check_invariant()` (lock-free read from anywhere) |
| **Algo** | `shared.algo_states` | `Mutex<[u8; N]>` | plain assignment under the guard | The Tanenbaum algorithm only |

### The fix
```rust
state[pid] = HUNGRY;          // algo state
// ... outside the lock:
shared.states[pid].store(HUNGRY, Ordering::SeqCst);   // oracle
```

### The bigger lesson
The oracle is a *witness* — set around the eat-section, never read by the algorithm. The algo state is what the algorithm reasons over. Conflating them produces a subtle bug: if `state[pid] = HUNGRY` is replaced with the oracle store, the *algo* state stays at THINKING (initial), `tanenbaum_test` reads `state[pid] != HUNGRY` and does nothing, no permit is deposited, and `notified().await` blocks forever on the first iteration. **Hangs at runtime, no compile error.**

---

## Mistake 9: `notified().await` *inside* the lock = self-deadlock

### What I wrote
```rust
{
    let mut state = shared.algo_states.lock().await;
    state[pid] = HUNGRY;
    shared.tanenbaum_test(&mut state, pid);
    shared.notify[pid].notified().await;     // ❌ awaiting under the lock
}
```

### What's wrong
If `tanenbaum_test` doesn't immediately put me into EATING (a neighbor is currently eating), I park at `.notified().await` *while still holding `algo_states`*. The only way for a neighbor to wake me up is to call `put_forks`, which begins with `shared.algo_states.lock().await`. They block forever on a lock I'll never release. Deadlock — every philosopher hangs at their first `notified().await`.

### The fix
Drop the lock *before* awaiting:

```rust
{
    let mut state = shared.algo_states.lock().await;
    state[pid] = HUNGRY;
    shared.tanenbaum_test(&mut state, pid);
}   // <-- lock released here
shared.notify[pid].notified().await;   // <-- await OUTSIDE the lock
```

### Why this is safe even with the gap
There's a window between the `}` (lock released) and `.notified().await` (subscription registered). What if a neighbor signals during that window? It still works:

- If `tanenbaum_test` set `state[pid] = EATING` and called `notify_one()` *before* I reached `.notified().await`, the permit is **deposited**. `notified().await` consumes it without blocking.
- If neighbors weren't ready, `tanenbaum_test` did nothing → no permit → `.notified().await` blocks until a neighbor's `put_forks` calls `tanenbaum_test(state, me)` and notifies me.

That deposit-without-waiter property of `Notify::notify_one()` is exactly what collapses Phase A and Phase B into one line of code.

### The general rule for tokio
> **Don't `await` while holding a `tokio::sync::Mutex` guard on data that the awaiter is waiting for the mutex-holder's neighbor to update.**

Awaiting under the lock is *technically* allowed by tokio (`MutexGuard` is `Send`, the runtime can park you), but it's almost always a bug — you're freezing every other task that needs the same lock. For this specific case it's not just a perf issue, it's a guaranteed deadlock.

---

## Mistake 10: the semaphore refill mechanism — and when to opt out with `.forget()`

This is the most important conceptual lesson from the dining-philosophers Rust port. It also explains why footman and barrier use the *same* `Semaphore` type but in opposite ways.

### What actually happens, mechanically

`SemaphorePermit` is a Rust struct with a `Drop` implementation. The Drop impl contains, roughly:

```rust
impl Drop for SemaphorePermit<'_> {
    fn drop(&mut self) {
        self.sem.add_permits(1);
    }
}
```

That's the whole "refill" mechanism. It's not magic, it's not convention — it's a method tokio's authors wrote, that runs when the permit value is destroyed.

So when you write:
```rust
let permit = sem.acquire().await.unwrap();   // count: N → N-1
// … do stuff …
}   // permit goes out of scope, Drop runs, count: N-1 → N
```

The "refill" is literally the Drop impl calling `add_permits(1)` on the way out. Hardcoded line of Rust code in tokio's source.

### What `.forget()` does

`.forget()` is a method on `SemaphorePermit` that consumes the permit (takes it by value) but skips the Drop. Roughly:

```rust
impl SemaphorePermit<'_> {
    pub fn forget(mut self) {
        // Mark the permit as "already handled" so Drop won't refund it.
    }
}
```

After `.forget()`, the permit value is gone, and `add_permits(1)` was never called. The semaphore's count stays at `N-1` instead of going back to `N`.

### Footman (resource pool) — Drop is what you want

```rust
let footman = Semaphore::new(N - 1);   // init capacity, ONCE

// philosopher task:
let permit = footman.acquire().await.unwrap();   // count: 4 → 3
eat().await;
// permit drops at end of scope → Drop → add_permits(1) → count: 3 → 4
```

You call `Semaphore::new(N-1)` once at construction. After that, you **never call `add_permits` again**. The count just oscillates between 0 and 4 forever as philosophers come and go, with Drop doing all the restocking.

The appeal: set capacity once, RAII handles accounting for free. Even if a philosopher's task panics mid-meal, Rust runs the Drop impl on unwind and the seat gets freed. Cannot leak a permit by accident.

### Barrier (one-shot signal) — Drop is what you fight

```rust
let gate = Semaphore::new(0);   // init at zero — gate starts closed

// in the barrier round:
gate.add_permits(N);                       // last arrival opens the gate
gate.acquire().await.unwrap().forget();    // each thread walks through and consumes the ticket
```

Here `add_permits` is called *every round* by whichever thread happens to be last, and `.forget()` prevents Drop from undoing that work. Without `.forget()`:
1. N threads each acquire a permit → count: N → 0
2. End of scope on each thread → Drop → `add_permits(1)` × N → count: 0 → N
3. Round 2 begins with N permits sitting there → threads breeze through the "closed" gate without waiting → barrier broken.

### The dichotomy

| Idiom | Init | `add_permits` | `forget()`? |
|---|---|---|---|
| **Resource pool** (footman, connection pool, rate limiter) | `new(capacity)` | never | no — let Drop refund |
| **Gate / one-shot signal** (barrier, broadcast release) | `new(0)` | every round | yes — consume the ticket |

Both use the *same* `tokio::sync::Semaphore`. They differ only in **what the count means**:

- Pool: "how many resources are free right now." Count must oscillate, RAII keeps it honest.
- Gate: "how many tickets are available for the next round." Tickets are consumed on use, never refunded.

The choice between letting Drop run or calling `.forget()` is the choice between these two semantics. Pick wrong and the program either livelocks (gate forgets to close) or leaks capacity (pool forgets to refill).

---

## Bonus: drop order matters for correctness

In `eat_footman`:

```rust
let _diner = shared.num_eaters.acquire().await.unwrap();   // declared 1st
let _left_chopstick = shared.chopsticks[left].lock().await;   // 2nd
let _right_chopstick = shared.chopsticks[right].lock().await;   // 3rd
// …
}   // LIFO drop: right → left → diner
```

The end-of-scope drop runs in **reverse declaration order** (LIFO):
1. `_right_chopstick` drops → right chopstick released.
2. `_left_chopstick` drops → left chopstick released.
3. `_diner` drops → semaphore permit returned, footman lets the next philosopher in.

This is the order you want — you put down chopsticks *before* leaving the table. Flipping it (releasing the seat first while still holding chopsticks) wastes capacity but isn't incorrect. Rust gives you the right order for free, just by writing the declarations in the natural order.

This LIFO-drop semantics is the same reason `_l` then `_r` in `eat_naive` releases right-then-left. You don't need explicit `release()` calls anywhere — the *order of `let` bindings* encodes the release order.

---

## Summary: the Rust-specific muscle memory

After this problem, the patterns I want to internalize:

1. **`let mut state = m.lock().await;`** — always a named binding, never an unbound expression.
2. **`if/else`, `loop`, `match` are expressions** — return values from them with `break (a, b)` or arm bodies, destructure with `let (a, b) = …`.
3. **`tokio::sync::Mutex::lock()` is async, not fallible** — `.lock().await` returns the guard, no `Result`.
4. **`tokio::sync::Mutex::try_lock()` is sync and fallible** — returns `Result<Guard, TryLockError>`.
5. **Two layers of state in algorithm code**: oracle (atomic, witness-only) and algo (plain types under one mutex, decision-making). Don't conflate APIs.
6. **Never `await` under a tokio mutex** when the awaited event depends on someone else acquiring the same mutex.
7. **`SemaphorePermit::Drop` refunds the permit** by calling `add_permits(1)`. `forget()` opts out. Pool ↔ Drop. Gate ↔ forget. Same primitive, opposite semantics.
8. **LIFO drop order is your release order** — write declarations in the order you want acquires, the compiler runs releases in reverse for free.
