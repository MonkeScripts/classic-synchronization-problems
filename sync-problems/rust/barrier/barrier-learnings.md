# Barrier in Rust: Mistakes & Learnings

Companion to `cpp/barrier/barrier-learnings.md` and `go/barrier/barrier-learnings.md`. Two variants live in this crate:

- `barrier/src/main.rs` — `Mutex` + `Condvar` + generation counter (the classic, same shape as `MyBarrierCond` in C++ and `barrier_cond/main.go` in Go).
- `barrier/src/bin/tokio.rs` — preloaded turnstile using `tokio::sync::Semaphore`. Slide-22 shape from the lectures, but on tokio's async runtime since `std` has no semaphore.

Both surfaced different Rust-specific friction. The tokio variant taught me about `.await` syntax, RAII permits, and `.forget()`. The cond variant taught me how unforgiving Rust is about lock-guard semantics — where the data lives, how to read it, how to drop it, and what the compiler will yell at you for trying.

---

## The journey at a glance

Stages 1–4 came from `tokio.rs`. Stages 5–10 are from `main.rs` (the cond version).

| # | Mistake | What it taught me |
|---|---|---|
| 1 | `acquire().await()` with parentheses | `.await` is **postfix syntax**, not a method call. `expr.await`, never `expr.await()` |
| 2 | `acquire().await.forget()` (no `.unwrap()`) | `Semaphore::acquire` returns `Result<SemaphorePermit, AcquireError>`. The error case only fires after `close()` — but the type system still makes you handle it |
| 3 | Missing semicolon on `state.count -= 1` | Rust's grammar parses the next `if` as the *expression value* of the previous statement unless terminated. `add_permits` calls inside an `if` *can* skip the semi (block returns `()`), but bare statements cannot |
| 4 | "Do I need `.forget()` here? I think it just dies" | A `SemaphorePermit` is an **RAII lease**, not a consumed token. Drop auto-returns the permit. Without `.forget()`, the gate never empties and round 2 races through |
| 5 | `self.state.count = 0` to update the field inside the mutex | `Mutex<T>` doesn't expose `T`'s fields. The only way to reach the data is through the lock guard |
| 6 | `let mut count = state.count; count += 1;` | `usize` is `Copy` — the let made an independent integer. Mutate via the guard or no other thread sees the increment |
| 7 | `wait_while(self.state.lock().unwrap(), …)` | `std::sync::Mutex` is **not reentrant**. `wait_while` *takes* the guard you already hold, doesn't lock again |
| 8 | Predicate written as `gen != s.generation` | C++ `cv.wait` and Rust `wait_while` use **opposite** predicate conventions. Read the function name before the body |
| 9 | Closure parameter named `gen` | Shadowed the captured snapshot. The closure receives `&mut BarrierState`, not `usize` |
| 10 | `let _ = wait_while(...).unwrap();` | `let _ = guard` drops the guard *immediately* and trips `let_underscore_lock`. The clean form is `let _guard = ...;` (named binding defers drop to end of scope) |

---

## Mistake 1: `.await()` with parentheses

### What I wrote
```rust
self.t1.acquire().await().forget()
```

### What the compiler said
```
error: incorrect use of `await`
   --> barrier/src/bin/tokio.rs:104:32
    |
104 |         self.t1.acquire().await().forget()
    |                                ^^
    |
help: `await` is not a method call, remove the parentheses
```

### The fix
```rust
self.t1.acquire().await.forget()
```

### The bigger lesson
`.await` is one of two **postfix-keyword expressions** in Rust — the other is `?`. Both look like methods but are syntax. Rust chose postfix specifically so async code chains left-to-right (`fetch().await.parse()` reads better than `parse(fetch().await)`). The trade-off is that the keyword is unparenthesized, which always feels wrong on first sight. It isn't.

---

## Mistake 2: forgot `acquire()` returns `Result`

### What I wrote
```rust
self.t1.acquire().await.forget()   // tries to call .forget() on Result
```

### What the compiler said
```
error[E0308]: mismatched types
   |
   |         self.t2.acquire().await
   |         ^^^^^^^^^^^^^^^^^^^^^^^ expected `()`, found `Result<SemaphorePermit<'_>, ...>`
```

### The fix
```rust
self.t1.acquire().await.unwrap().forget();
```

### Why is it a `Result`?
`tokio::sync::Semaphore` can be **closed** (`Semaphore::close`), at which point `acquire()` returns `Err(AcquireError(()))`. Closing is how tokio signals "the producer is gone, give up." For our barrier we never close, so `unwrap()` is the right call — but the API surface still forces you to acknowledge the case. That's a real Rust theme: **the type system makes the unhappy path syntactically visible** even when your invariants rule it out.

---

## Mistake 3: missing semicolon

### What I wrote
```rust
let mut state = self.state.lock().unwrap();
state.count -= 1
if state.count == 0 {                  // <-- compiler is angry here
    self.t2.add_permits(self.expected);
}
```

### What the compiler said
```
error: expected `;`, found keyword `if`
   |
   |             state.count -= 1
   |                             ^ help: add `;` here
```

### Why some lines need a semi and others don't

I had this elsewhere too:
```rust
if state.count == self.expected {
    self.t1.add_permits(self.expected)   // no semi — fine
}
```
Why is *this* OK? Because the `if` block is the *last expression of the enclosing block*, and its type is `()`. The block evaluates to `()`, the outer block evaluates to `()`, no statement-terminator needed.

But `state.count -= 1` is a **statement** in the middle of a block, and it must terminate before the next thing. Rule of thumb: **every statement that isn't the last thing in its block needs a `;`**. The "implicit return" only applies to the trailing expression.

---

## Mistake 4: thinking the permit "just dies"

### What I wrote
```rust
self.t2.acquire().await // Do i need to forget here? I think it just dies
```

### Why "it just dies" is the wrong intuition

`tokio::sync::SemaphorePermit` has a `Drop` impl that calls `add_permits(1)` to **return the permit** to the semaphore. Drop here means *hand it back*, not *destroy it*. So:

```
acquire().await   →  semaphore -= 1     (one permit consumed)
permit drops      →  semaphore += 1     (permit returned)
                     ────────────────
net effect        =  0
```

For the barrier this is catastrophic. Trace round 1's Phase 2 with N=8 and **no `.forget()`**:

| Step | t2 |
|---|---|
| Round start | 0 |
| Last thread: `add_permits(8)` | 8 |
| All 8 acquire (consume) | 0 |
| All 8 permits drop (return) | 8 |
| **Round 1 end** | **8** |

Round 2's Phase 2 starts with t2 already saturated. The first thread to `acquire()` doesn't block — even though no one else has decremented `count` yet — and it races into round 3 while others are still finishing round 2. The `GLOBAL_ROUND` invariant assert fires on round 2 (or later, depending on scheduling), not round 1, which is the worst kind of bug to debug.

### The fix
```rust
self.t2.acquire().await.unwrap().forget();
```
`.forget()` consumes the `SemaphorePermit` without running its destructor — the permit is gone permanently, the gate empties as intended, the cycle is `0 → N → 0` per round.

### The mental model

- `add_permits(N)` ≈ `release(N)` — issues new permits.
- `acquire().await` returns an **RAII lease**, not a token. The contract is "I will return this when I'm done."
- `.forget()` opts out of that contract: "I'm consuming this permanently."

So `add_permits` and `forget` are the symmetric pair when you want classic counting-semaphore semantics. The default (lease, not consume) is the right one for capacity-bounding use cases — rate limiters, connection pools — where you genuinely want the permit back when the work finishes. Barrier-style broadcasts, where the gate must drain, are the unusual case, and tokio makes you say so explicitly.

---

## Mistake 5: trying to read/write fields through the `Mutex` itself

### What I wrote
```rust
let mut state = self.state.lock().unwrap();
// ...
self.state.count = 0;            // ← compile error
self.state.generation += 1;      // ← compile error
```

### Why it doesn't compile
`self.state` has type `Mutex<BarrierState>`. The `Mutex` type *deliberately* doesn't expose `.count` or `.generation` — that's the whole point of having the wrapper. The only way to reach the inner data is through the lock guard.

### The mental model
A `Mutex` is a locked safe. `self.state` is the safe itself. `self.state.lock()` is opening it; the returned `MutexGuard` is the open-safe handle. Inside the function I have *one* open safe — named `state` — and `state.count` is "reach into the open safe and touch the count field." That's the only way.

### The fix
```rust
state.count = 0;
state.generation += 1;
```
The `MutexGuard<T>` `Deref`s to `&T` and `DerefMut` to `&mut T`, so `state.count` desugars to `(*state).count` — that *is* the field of the inner `BarrierState`, i.e., the field inside the mutex.

### The bigger lesson
Once you've locked, **forget about `self.state` for the rest of the function.** Your only handle to the data is the guard. If the naming is confusing, rename: `let mut data = self.state.lock().unwrap();` makes `data.count = 0;` read more cleanly as "set the field inside the locked data."

---

## Mistake 6: the local-copy trap

### What I wrote
```rust
let mut state = self.state.lock().unwrap();
let mut count = state.count;     // copies the integer out of the guard
count += 1;                       // increments the LOCAL copy
if count == self.expected { ... } // checks the local copy
```

### Why it deadlocks
`usize` is `Copy`. `let count = state.count;` makes an independent integer. Increments to `count` never touch `state.count`. Picture N=8 workers: every thread reads `state.count == 0`, copies it as `count = 0`, increments locally to 1. **No thread ever sees count reach 8.** The if-condition is never true; nobody trips the barrier; everyone falls into `wait_while`. Lockstep deadlock.

### The fix
```rust
state.count += 1;
if state.count == self.expected { ... }
```
One line, no local, the increment lands on the shared field where every other thread can see it.

### The bigger lesson
Rust's `Copy` types make **reads silently produce independent values.** That's usually exactly what you want — small types are cheap to copy, no aliasing surprises. But for shared-state mutation it's a footgun: the language *won't* warn you that your increments aren't affecting the shared field, because at the type level you really did just copy a `usize`. The discipline is "if I'm going to write back, mutate through the guard, don't go through a local."

---

## Mistake 7: locking twice from the same thread

### What I wrote
```rust
let mut state = self.state.lock().unwrap();         // line 44: holding the lock
// ...
self.cv.wait_while(self.state.lock().unwrap(), …)   // line 53: ASKS FOR THE SAME LOCK
```

### Why it deadlocks
`std::sync::Mutex` is **not reentrant**. The second `lock()` waits for the first to release. The first never releases because *I'm* the one holding it. Hang forever.

### Why `wait_while` doesn't need a fresh lock
`Condvar::wait_while` does three things atomically: takes your guard, releases the lock + sleeps until notified, re-acquires the lock and hands a fresh guard back. **It needs your existing guard so it can release-and-sleep without a missed-wakeup window.** Asking it to lock for me would defeat the whole atomic-release-and-sleep guarantee.

### The fix
```rust
self.cv.wait_while(state, |s| gen == s.generation)
```
Pass `state` directly. The compiler then says "value moved" if you try to use `state` again — that's correct, ownership has transferred to `wait_while`, which gives me a new guard via the return value.

### The bigger lesson
Rust's standard mutex deliberately doesn't support reentrant locking. (`parking_lot::ReentrantMutex` exists but it's niche.) That forces you to think clearly about *who holds the lock and where*, instead of papering over a tangle of nested locks. For condvars specifically, the contract "give me your guard, I'll give you one back" is the *whole* point — that's how the atomic release-and-sleep is achieved.

---

## Mistake 8: predicate inversion (C++ vs Rust)

### What I wrote
```rust
self.cv.wait_while(state, |s| gen != s.generation)
```

### Why it's the wrong polarity

C++ and Rust use **opposite predicate conventions:**

| Library | Call | Predicate semantics |
|---|---|---|
| C++ | `cv.wait(lock, pred)` | wait *until* `pred` is **true** |
| Rust | `cv.wait_while(guard, pred)` | wait *while* `pred` is **true** |

In C++ I had `cv.wait(lk, [&]{ return gen != generation_; })` — wait until generation moves. Translating that body verbatim to Rust flips the meaning: `wait_while(guard, |s| gen != s.generation)` says "keep sleeping while the generation has already moved" — i.e., release everyone exactly when they shouldn't be released.

### The fix
```rust
self.cv.wait_while(state, |s| gen == s.generation)
```
"Keep waiting while the generation hasn't moved."

### The bigger lesson
The function name is the warning: **`wait_while`** literally tells you "while what is true, keep sleeping." Rust also has the older `wait` and `wait_timeout`; `wait_while` is the predicate-loop form, deliberately named for the polarity. Whenever you cross language boundaries with cv-wait-with-predicate, mentally translate by reading the *function name* before the *predicate body*.

---

## Mistake 9: shadowing the snapshot with the closure parameter

### What I wrote
```rust
let gen = state.generation;
// ...
self.cv.wait_while(state, |gen| gen != self.state.generation)
//                          ^^^ this shadows the outer `gen`
```

### Why it doesn't even type-check
The closure receives `&mut BarrierState`, not a `usize`. By naming the parameter `gen`, the body's `gen` refers to the `&mut BarrierState`, and you're trying to compare a guard reference against a Mutex's `.generation` field, which (per Mistake 5) doesn't exist anyway.

### The fix
Pick a parameter name that doesn't collide:
```rust
self.cv.wait_while(state, |s| gen == s.generation)
```
Now `gen` resolves to the captured outer snapshot, and `s.generation` reads the current generation through the closure's `&mut BarrierState`.

### The bigger lesson
Rust's variable-shadowing rules are *generous* — you can rebind any name in any inner scope. That's usually convenient, but in closures it can silently swap a captured variable for a parameter of the same name, and the type error you get back can be confusing because it's about a name you thought you knew the type of. When in doubt, give closure parameters short, ugly names (`s`, `g`) so they can't possibly shadow your captures.

---

## Mistake 10: the `let _ = lock-guard` lint dance

After getting the wait branch right, I had to discard the value `wait_while` returns. This took *three* tries.

### Attempt 1: trailing expression
```rust
self.cv.wait_while(state, |s| gen == s.generation)
```
Type mismatch — function returns `()`, expression is `LockResult<MutexGuard<...>>`.

### Attempt 2: bare semicolon
```rust
self.cv.wait_while(state, |s| { gen == s.generation });
```
Type-checks, but warning: ``unused `Result` that must be used``. `LockResult` carries `#[must_use]`.

### Attempt 3: `let _ =` per the compiler hint
```rust
let _ = self.cv.wait_while(state, |s| { gen == s.generation }).unwrap();
```
**Hard error:**
```
error: non-binding let on a synchronization lock
       this lock is not assigned to a binding and is immediately dropped
       #[deny(let_underscore_lock)] (part of #[deny(let_underscore)]) on by default
```

### Attempt 4: drop the `let _ =`, keep `.unwrap()`
```rust
self.cv.wait_while(state, |s| { gen == s.generation }).unwrap();
```
**New warning:** ``unused `std::sync::MutexGuard` that must be used. if unused the Mutex will immediately unlock``.

### What finally worked: named binding
```rust
let _guard = self.cv.wait_while(state, |s| gen == s.generation).unwrap();
```

### Why this passes both lints

The two lints exist for *different* mistakes:

| Form | Drop timing | Lint that fires |
|---|---|---|
| `let _ = expr;` | **immediately** at the `let` | `let_underscore_lock` (if expr is a sync guard) |
| `let _x = expr;` | end of enclosing scope | none |
| `expr;` (statement) | immediately at the `;` | `unused_must_use` (if type is `#[must_use]`) |
| `drop(expr);` | immediately, explicitly | none |

`let_underscore_lock` exists because `let _ = mutex.lock();` is a famous footgun — people think they're holding the lock for the scope, but `let _` releases it instantly. Naming the binding (`_guard`, `_g`, anything) defers the drop to end of scope, which is what you almost always want when working with locks.

### The bigger lesson
**Rust's compiler is unusually loud about lock semantics on purpose.** Concurrency bugs are silent at runtime, so Rust tries to make them loud at compile time. The takeaway:

- Want the lock held to end-of-scope? `let _guard = ...;` (named, even if unused).
- Want to release it right now? `drop(...);`.
- Want to discard a Result that contains a guard? `let _ = ...;` *only if* you don't `.unwrap()` it (so the type is `Result`, not the guard itself).

The named-binding form is the cleanest fit for the wait branch of a barrier: I'm done with the guard, don't need its value, but want it to drop *naturally at function end* like every other local — and let the compiler stop yelling.

---

## Why I used `std::sync::Mutex` inside an async function

The state mutex (`Mutex<BarrierState>`) is `std::sync::Mutex`, not `tokio::sync::Mutex`. Tokio's docs explicitly recommend this when **the critical section never `.await`s**. The std mutex is faster (it's just `parking_lot`-style spinning underneath) and the only thing the tokio mutex buys you is the ability to hold the guard across an await — which our code deliberately doesn't do, hence the inner `{ ... }` blocks that drop the guard before the `t1.acquire().await`.

If I held a `std::sync::MutexGuard` across `.await`, two things go wrong:
1. The guard is `!Send`, so the future containing it is `!Send`, so `tokio::spawn` (which requires `Send` futures) fails to compile.
2. Even if I worked around that, the await yields the executor while holding a blocking lock — any other task that touched the mutex would deadlock the worker thread.

The `{ }` scope discipline is how you statically prove you're not doing either.

---

## Two binaries in one Cargo crate

Both variants live under `barrier/`, sharing `Cargo.toml`. Cargo auto-discovers `src/bin/*.rs` as additional binary targets, so the layout is:

```
barrier/
  Cargo.toml          # depends on tokio (only used by the bin variant)
  src/
    main.rs           # cond + generation     →  cargo run -p barrier
    bin/
      tokio.rs        # preloaded semaphore   →  cargo run -p barrier --bin tokio
```

This was the lightest-weight way to get two runnable variants without forcing a sibling crate or feature gates. The tokio dependency adds compile time to **both** bins, but only the `tokio.rs` bin actually links the runtime.

---

## When to pick which variant

| Variant | Lines of meaningful code | Where the friction was | Failure mode if wrong |
|---|---|---|---|
| `Mutex` + `Condvar` + generation (`main.rs`) | ~10 | guard vs Mutex distinction, Copy-type local-copy trap, lock-aware lints | type errors at compile time; if mistakes pass type-check (e.g. local-copy), N-thread deadlock |
| `tokio::sync::Semaphore` preloaded turnstile (`tokio.rs`) | ~15 | postfix `.await`, RAII `SemaphorePermit`, `.forget()` to consume, no-await-while-locked | invariant assert fires on round 2+ if `.forget()` is missed |

The cond version is what I'd write in production for a sync barrier. The tokio version is the natural fit only when the rest of the codebase is already async.

**The recurring meta-lesson** across both variants is that Rust pushes concurrency mistakes from runtime to compile time *more aggressively than any other language I've used here*. The C++ domino barrier compiled and ran on the first try with a wrong line; the Go version compiled but panicked on round 2 with a WaitGroup-reuse error. The Rust version refused to compile until the lock-guard-vs-mutex distinction was in my hands; even after that, the compiler kept firing lints (`let_underscore_lock`, `unused_must_use`, the `MutexGuard` `must_use`) until the discard was done in a way the language deemed unambiguous. That's not friction for friction's sake — those lints are encoding "places where humans write barrier-class bugs." It just feels heavier than the other two languages because you pay the cost up front.

---

<!-- Add stages below as you hit and understand new bugs. -->
