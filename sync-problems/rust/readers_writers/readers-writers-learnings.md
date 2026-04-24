# Readers-Writers (Rust, async/tokio): Mistakes & Learnings

A record of implementing the readers-writers KV-cache in async Rust with tokio — two flavors (`std::sync::RwLock` baseline, hand-rolled Lightswitch + turnstile using `tokio::sync::Semaphore`) — the bugs along the way, and the closing piece of the three-language fairness story.

---

## The headline observation

Three families of takeaway, in ascending order of transferability:

1. **Async rewrite mechanics.** `#[async_trait]`, `#[tokio::main]`, `tokio::spawn`, `.await` — these aren't just syntax changes. They force you to think about which values live across which `.await` points, and whether those values are `Send`. Many bugs I'd normally never hit in sync Rust surfaced because of this.

2. **The tokio permit RAII model** and specifically the `permit.forget()` + `add_permits()` dance for permits whose lifetime crosses a function/task boundary. This is the async-Rust answer to "how do I express a C-style semaphore in Rust's RAII world?" Non-obvious the first time; obvious in hindsight.

3. **The three-language fairness conclusion.** The closing step of the hypothesis that started with the flaky C++ turnstile. C++ `std::counting_semaphore` (no FIFO promise) → ~85% pass. Go channels (FIFO guaranteed) → 500/500. Rust `tokio::sync::Semaphore` (FIFO guaranteed) → 500/500. Same algorithm, different primitives, cleanly divided outcomes. The primitive's wakeup-ordering guarantee is the hinge.

---

## The journey at a glance

| # | Mistake | Fix | Category |
|---|---|---|---|
| 1 | Called `.release()` on `tokio::sync::Semaphore` — doesn't exist | Bind the permit with `let _x = sem.acquire().await.unwrap();` — it releases when dropped (RAII). Or use `sem.add_permits(1)` for the forgotten-permit pattern. | tokio API |
| 2 | Called `.acquire()` without `.await` — returns an unstarted `Future`, nothing happens | `.acquire().await.unwrap()` — three method calls, all required | async Rust |
| 3 | Compound-op race on `rc`: `self.rc.fetch_add(1, ...); if self.rc == 1 { ... }` — two separate atomic ops, not atomic as a group (same bug as C++) | Use `fetch_add`'s return value: `let new_rc = self.rc.fetch_add(1, Ordering::SeqCst) + 1;` then `if new_rc == 1` | atomics |
| 4 | Compared `AtomicI32` directly: `if self.rc == 1` | Can't compare atomic to `i32`. But don't fix it that way — use the return value of fetch_add (see #3). | atomics |
| 5 | Forgot to `permit.forget()` on first-reader entry — the permit auto-dropped at scope end, releasing room_empty before the last reader exited | After acquiring: `permit.forget()` — consumes the permit WITHOUT triggering its Drop impl | tokio permits |
| 6 | Forgot to `add_permits(1)` on last-reader exit — no way to get the "forgotten" permit back otherwise | `self.room_empty.add_permits(1)` — manually puts one back into the semaphore's count | tokio permits |
| 7 | Typo: `_new.rc == 0` where it should be `_new_rc == 0` — underscore became a dot | Read what the compiler points at; `no field rc on type i32` is a very specific message | typo |
| 8 | `std::sync::MutexGuard` held (even briefly) across an `.await` — MutexGuard is `!Send`, so the whole `async fn`'s future becomes `!Send`, which `tokio::spawn` rejects | Tight-scope the guard in an inner block that ends *before* the next `.await`. `drop(guard)` does not help — see deep-dive below. | async+Send |
| 9 | `let out = { ...; expr; };` with trailing semicolon on `expr` → block evaluates to `()` | Last expression in a block gets NO `;` if it's to be the block's value (same bug as from the mpsc Rust exercise) | Rust syntax |
| 10 | `let out = { ... }` with no `;` after the closing `}` → parser treats following statements as expression continuation | Every `let` statement needs its terminating `;`, even when the RHS is a block | Rust syntax |
| 11 | C-style parens: `if (cond) { ... }` | Rust doesn't need them; emits `unused_parens` warning. `cargo fix --allow-dirty` strips them. | style |
| 12 | Forgot to uncomment `run_benchmark("HandRolled", ...)` in `main` | Grep for `TODO` / `//` after each exercise | oversight |

Three themes bind these:
- **C++/Go muscle memory leakage** (#1, #4, #11) — "I'll just port my C++ code" fails because the Rust/tokio API is shaped differently.
- **Compound-op confusion** (#3, #4) — the #1 transferable bug from C++, still present in Rust.
- **Async-specific traps** (#8, most of #5/#6) — things that only happen because you're crossing `.await` points.

---

## Deep dive: the tokio `Semaphore` RAII model

This is the one tokio idiom that took me longest to internalise.

### Acquire returns a permit; drop releases it

```rust
let sem = Semaphore::new(1);
{
    let permit = sem.acquire().await.unwrap();
    // hold ...
}  // permit drops here → sem's internal count incremented
```

No `.release()` method. None. The whole point of RAII is that "release" happens for free at scope exit. Rust + tokio designed this deliberately: the #1 class of C++ semaphore bugs is "forgot to release on error path" — RAII makes that impossible.

Three call patterns you'll use:

| Pattern | Use case | Code |
|---|---|---|
| Bind-and-drop-at-scope | Short critical section | `{ let _p = sem.acquire().await.unwrap(); ... }` |
| Bind-and-drop-at-fn-exit | Whole-function hold (writer) | `let _p = sem.acquire().await.unwrap();` at fn start — drops at fn return |
| Acquire-and-immediate-drop | "Gate-pass" (reader turnstile) | `drop(sem.acquire().await.unwrap());` |

All three are different uses of the same API. No different method calls, just different binding lifetimes.

### LIFO scope-exit drop gives you reverse-acquisition release for free

For the writer:
```rust
async fn set(...) {
    let _turnstile = self.turnstile.acquire().await.unwrap();
    let _room      = self.room_empty.acquire().await.unwrap();
    // ... write ...
}  // Dropping in reverse declaration order: _room, then _turnstile
```

Rust drops local bindings in LIFO order at scope end. That's exactly reverse-acquisition-order, which is the conventional release order for nested locks. **You get it for free** — no explicit release calls, no ordering mistakes possible. This is genuinely nicer than the C++ or Go versions where you had to type the reverse order yourself.

### The forgotten-permit dance (when RAII breaks down)

RAII works when *the same function* acquires and releases. It doesn't work when *different functions (or tasks) do*.

Readers-writers has exactly that pattern: the **first reader** acquires `room_empty`, a **different last reader** releases it. RAII can't express this directly — the permit is bound to a local, and the local has to go out of scope somewhere.

Tokio's answer: break the RAII contract explicitly.

```rust
// First reader:
let permit = self.room_empty.acquire().await.unwrap();
permit.forget();   // ← consumes the permit without running its Drop impl
                   //   The semaphore's count stays decremented.
                   //   The permit value is gone, but its "side effect" persists.

// ... much later, in a completely different reader's task ...

// Last reader:
self.room_empty.add_permits(1);   // ← manually puts a permit back
```

`forget()` is **explicitly safe** — it's a deliberate API for exactly this pattern. It's not a hack. The tokio docs call it out:

> [After `forget()`] the caller ... is responsible for eventually releasing the permit by calling [`Semaphore::add_permits`].

So the contract becomes: instead of "acquire and release MUST be a matched RAII pair," it becomes "every `forget` MUST be matched by an `add_permits(1)`, possibly from a different task." Same correctness property, looser temporal binding.

### Why this exists: tokio vs std::sync::Mutex

Compare with how you'd write first-reader/last-reader with `std::sync::Mutex` alone:

```rust
let first_reader = { let mut rc = rc_mu.lock().unwrap(); *rc += 1; *rc == 1 };
if first_reader { /* acquire room_empty somehow */ }
```

In std::sync::Mutex, the guard drops automatically, no special action needed, because the mutex lock is always acquired and released by the same thread. `tokio::sync::Semaphore` is more flexible — permits *can* be transferred (via `acquire_owned`), sent across tasks, or forgotten entirely — at the cost of losing the strict RAII invariant. The `forget()` + `add_permits()` pattern is the escape hatch that recovers C-style semaphore semantics when you need them.

---

## Deep dive: the `!Send` MutexGuard across `.await` trap

Under `tokio::spawn` with the multi-thread runtime, every Future needs to be `Send` — the runtime can move the task to a different OS thread at any `.await`. `std::sync::MutexGuard` is deliberately `!Send` (because std mutexes are thread-bound on many platforms). So holding one across an `.await` poisons the whole Future's Send-ness.

My first attempt:
```rust
self.counters.enter_read();
let guard = self.map.lock().unwrap();
let out = guard.get(_k).cloned().unwrap_or_default();
drop(guard);                                          // "explicit drop"
self.counters.exit_read();
{
    let _bibo = self.bibo.acquire().await.unwrap();   // ← error here
    ...
}
```

Compile error:
```
future is not `Send`
MutexGuard<...> is not Send
future is not `Send` as this value is used across an await
```

### Why `drop(guard)` doesn't help

The `Send` auto-trait inference for `async fn`s is done by inspecting the generated state machine and checking whether any `!Send` type is "live" at any `yield` (`.await`) point. The analysis is based on **lexical scope**, not on dataflow. Even though my `drop(guard)` executes before the `.await`, the *name* `guard` is still declared in the enclosing scope — the compiler's auto-trait analysis assumes `guard` *could* be used at the await, and conservatively flags it.

The fix is to tight-scope the guard in an inner block so it's *syntactically* out of scope before the await:

```rust
self.counters.enter_read();
let out = {
    let guard = self.map.lock().unwrap();
    guard.get(_k).cloned().unwrap_or_default()
};  // guard's lexical scope ends HERE. Send analysis is happy.
self.counters.exit_read();
{
    let _bibo = self.bibo.acquire().await.unwrap();
    ...
}
```

### The general rule

> **Every std::sync::Mutex lock inside an `async fn` on a multi-thread tokio runtime MUST be scoped in a block that ends before any subsequent `.await`.** Explicit `drop()` is not enough. If you need to hold a lock across an `.await`, switch to `tokio::sync::Mutex`.

### Why I used std::sync::Mutex anyway

For the map, the critical section is a single lookup or insert — tight scope, never across `.await`. std::sync::Mutex is faster and doesn't require a runtime to be present. Using tokio::sync::Mutex for a trivial, non-awaited lock would be all cost, no benefit.

The rule of thumb: **std::sync::Mutex for tight CPU-bound critical sections; tokio::sync::Mutex only when you need to hold across `.await`.**

---

## Deep dive: the cross-language fairness finding (closed)

This is the whole point of doing the problem in three languages.

### The complete picture

| Language | Primitive | FIFO wakeup guaranteed? | Turnstile stress result |
|---|---|---|---|
| C++ | `std::counting_semaphore<1>` | **No** (spec only guarantees "at least one" waiter unblocks) | ~85% pass, 15% timeout under stress |
| Go | `chan struct{}` cap 1 | **Yes** (spec: "served in FIFO order") | 500/500 clean |
| Rust | `tokio::sync::Semaphore` | **Yes** (docs: "This Semaphore is FIFO") | 500/500 clean |

Same algorithm (Downey slide 12). Same problem (readers-writers). Same benchmark (8R/2W/1000 ops). Only the primitive varies.

### Why this is a real conclusion, not just an observation

Observations are hard to trust in concurrency because timing varies. One flaky run is "bad luck"; 500 clean runs is "worked today." The strength of the three-language result is:

- C++ fails ~15% reliably — not transient, consistent.
- Go and Rust pass 500/500 — not lucky, consistent.
- The exact property that differs between them is documented in the respective specs.

That's as rigorous as this kind of empirical concurrency testing gets without formal verification.

### What Downey's textbook proof actually assumes

The classical "starve-free readers-writers using turnstile" proof in *The Little Book of Semaphores*:

> When a writer arrives, all subsequent readers queue behind the turnstile. The readers currently in the room will eventually finish. The writer then proceeds. Therefore writers cannot starve.

That argument relies on "the readers currently in the room will eventually finish." In practice, *eventually* means "the reader blocked on room_empty (waiting for a writer to release) gets woken when the writer releases." If the wakeup mechanism is non-FIFO, some readers can keep winning the race and never give the blocked reader a chance to see the signal.

The textbook proof **silently assumes fair scheduling**. It's not in the proof because in the textbook's model, waiters are picked randomly and "eventually" is guaranteed probabilistically. Real primitives use futexes or per-language mechanisms that can be arbitrarily unfair under adversarial scheduling.

### The transferable lesson

> **A concurrency algorithm is correct only up to the fairness guarantees of its primitives.** Before trusting a liveness proof, check what your runtime actually promises about wakeup ordering. The same algorithm can be reliable on one platform and flaky on another solely because of this.

This is the #1 lesson from the whole CS3211 readers-writers exercise. It transfers directly to any concurrent system where you care about progress guarantees, not just mutual exclusion.

---

## Deep dive: walking through the implementation

### The state

```rust
pub struct KVCacheHandRolled {
    map: Mutex<HashMap<String, String>>,       // std::sync — short critical sections, no await across
    counters: Counters,                         // invariant oracle (separate from solution state)
    turnstile: Semaphore,                       // cap 1 — no-starve gate
    room_empty: Semaphore,                      // cap 1 — held while readers or writer in room
    bibo: Semaphore,                            // cap 1 — mutex for rc manipulation
    rc: AtomicI32,                              // reader count; atomic only to satisfy Sync
}
```

- `map` is `std::sync::Mutex`, not `tokio::sync::Mutex`, deliberately. The lock scope is a single tight block — never held across `.await`. See the `!Send` trap deep-dive.
- `rc` is `AtomicI32` for the Rust type system (needs Sync). All rc manipulation is protected by `bibo`, so per-op atomicity isn't doing much work — it's just the simplest way to satisfy the type checker.
- Three `Semaphore`s — one per synchronization role, just like the C++ and Go versions.
- `Counters` is separate from `rc` by design — instrumentation oracle vs solution state, same argument as the C++ writeup.

### Reader (`get`) — annotated

```rust
async fn get(&self, k: &str) -> String {
    // Gate-pass: acquire the turnstile and immediately drop it.
    drop(self.turnstile.acquire().await.unwrap());

    // Entry under bibo.
    {
        let _bibo = self.bibo.acquire().await.unwrap();
        let new_rc = self.rc.fetch_add(1, Ordering::SeqCst) + 1;
        if new_rc == 1 {
            // First reader — acquire room_empty and FORGET.
            let permit = self.room_empty.acquire().await.unwrap();
            permit.forget();
        }
    }  // _bibo drops here, releasing

    // Actual read, outside bibo, inside room_empty.
    self.counters.enter_read();
    let out = {
        let guard = self.map.lock().unwrap();      // std mutex, tight block scope
        guard.get(k).cloned().unwrap_or_default()
    };
    self.counters.exit_read();

    // Exit under bibo.
    {
        let _bibo = self.bibo.acquire().await.unwrap();
        let new_rc = self.rc.fetch_sub(1, Ordering::SeqCst) - 1;
        if new_rc == 0 {
            // Last reader — put the forgotten permit back.
            self.room_empty.add_permits(1);
        }
    }

    out
}
```

Five notable decisions encoded here:

1. `turnstile` gate-pass on the first line — prevents starvation of writers.
2. `fetch_add` captures the return value (compound-op safety).
3. `permit.forget()` on the first reader — breaks the RAII contract deliberately.
4. `map.lock()` is tight-scoped so the `!Send` guard doesn't cross the `.await` on line after.
5. `add_permits(1)` on the last reader — the reciprocal of `forget()`.

### Writer (`set`) — annotated

```rust
async fn set(&self, k: String, v: String) {
    let _turnstile = self.turnstile.acquire().await.unwrap();
    let _room      = self.room_empty.acquire().await.unwrap();
    self.counters.enter_write();
    {
        let mut guard = self.map.lock().unwrap();
        guard.insert(k, v);
    }
    self.counters.exit_write();
    // _room drops first (LIFO), then _turnstile — free reverse-order release
}
```

Four lines of sync code. LIFO drop order handles release automatically. This is my favorite piece of the whole Rust implementation — the writer's three sync concerns (turnstile, room_empty, map mutex) are cleanly expressed, and the release ordering is structural.

---

## Final results

| Implementation | 500-run stress | Mean time (8R/2W/1000 ops) |
|---|---|---|
| `KVCacheRw` (`std::sync::RwLock`) | 500/500 ✓ | ~1-2 ms |
| `KVCacheHandRolled` (turnstile via tokio) | **500/500 ✓** | ~5-6 ms |

The baseline is ~3× faster — expected, `std::sync::RwLock` is heavily optimized for reader-heavy workloads with pure atomics, and tokio's Semaphore has real runtime overhead per `acquire().await`. For correctness/fairness validation (the actual point), both are fine.

---

## Working code (full class, for reference)

```rust
pub struct KVCacheHandRolled {
    map: Mutex<HashMap<String, String>>,
    counters: Counters,
    turnstile: Semaphore,
    room_empty: Semaphore,
    bibo: Semaphore,
    rc: AtomicI32,
}

impl KVCacheHandRolled {
    pub fn new() -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
            counters: Counters::new(),
            turnstile: Semaphore::new(1),
            room_empty: Semaphore::new(1),
            bibo: Semaphore::new(1),
            rc: AtomicI32::new(0),
        }
    }
}

#[async_trait]
impl Cache for KVCacheHandRolled {
    async fn get(&self, k: &str) -> String {
        drop(self.turnstile.acquire().await.unwrap());
        {
            let _bibo = self.bibo.acquire().await.unwrap();
            let new_rc = self.rc.fetch_add(1, Ordering::SeqCst) + 1;
            if new_rc == 1 {
                let permit = self.room_empty.acquire().await.unwrap();
                permit.forget();
            }
        }
        self.counters.enter_read();
        let out = {
            let guard = self.map.lock().unwrap();
            guard.get(k).cloned().unwrap_or_default()
        };
        self.counters.exit_read();
        {
            let _bibo = self.bibo.acquire().await.unwrap();
            let new_rc = self.rc.fetch_sub(1, Ordering::SeqCst) - 1;
            if new_rc == 0 {
                self.room_empty.add_permits(1);
            }
        }
        out
    }

    async fn set(&self, k: String, v: String) {
        let _turnstile = self.turnstile.acquire().await.unwrap();
        let _room      = self.room_empty.acquire().await.unwrap();
        self.counters.enter_write();
        {
            let mut guard = self.map.lock().unwrap();
            guard.insert(k, v);
        }
        self.counters.exit_write();
    }
}
```

---

## Three-language summary

The readers-writers exercise across C++, Go, and Rust doesn't teach one lesson — it teaches a nested set:

1. **At the algorithm level**: Lightswitch is reader-preference and starves writers; turnstile fixes it *in the abstract model*. Same everywhere.

2. **At the primitive level**: each language gave me a different synchronization primitive to express the same algorithm. C++ had `std::counting_semaphore`, Go had cap-1 `chan struct{}`, Rust had `tokio::sync::Semaphore`. The code ended up roughly the same length and roughly the same shape, modulo language idioms (RAII in Rust, `defer` in Go, scope-exit in C++).

3. **At the runtime level**: the algorithm's liveness depends on the primitive's wakeup fairness. C++ didn't promise FIFO; Go and Rust did. The same turnstile code was flaky in one and rock-solid in the other two, solely because of this.

4. **At the meta level**: concurrency textbook proofs silently assume fair scheduling. In practice, "fair" varies. Before claiming an algorithm starve-free, check what your primitive promises about wakeup ordering.

If I had to distill the whole three-language investigation into one sentence:

> **Correctness transfers; liveness doesn't — liveness is a joint property of the algorithm and the primitive's scheduler.**

That's the lesson I'd want any future-me doing concurrent-systems work to have internalized.

---

## Cross-references

- [`cpp/readers_writers/readers-writers-learnings.md`](../../cpp/readers_writers/readers-writers-learnings.md) — the original flakiness finding with the hypothesis.
- [`go/readers_writers/readers-writers-learnings.md`](../../go/readers_writers/readers-writers-learnings.md) — the first confirmation with FIFO channels.
- This doc — the second confirmation with FIFO tokio semaphore; closes the argument.

---

## Next moves

- [ ] **Update the C++ learnings doc** to reference this finding. Its "open item — hypothesized fairness gap" can now be marked "confirmed via Go + Rust cross-language testing."
- [ ] **Document the async `!Send` MutexGuard gotcha** somewhere more prominent — this is the kind of trap that'd bite future-me in any tokio project, not just readers-writers. Maybe a `rust/docs/async-gotchas.md` or similar.
- [ ] **Sabotage experiments to cement the tokio idioms:**
  - Remove `permit.forget()` → predict exactly what happens (permit auto-drops at scope, releasing room_empty while readers still in room, invariant fires).
  - Replace `std::sync::Mutex` with `tokio::sync::Mutex` on `map` and compare throughput — measures the runtime-overhead cost of the async-aware primitive.
  - Remove the turnstile acquire in the writer → predict the flakiness returns (but this time only under sustained reader load, and it'd be a Lightswitch-style starvation, not a fairness-caused livelock).
- [ ] **Plain `std::sync::Mutex` variant** (Impl 2 per the template's header comment) — three-line class, data point for "at what R:W ratio does RwLock beat Mutex?" Still outstanding for a complete writeup.
