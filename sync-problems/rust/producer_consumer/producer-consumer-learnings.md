# Producer-Consumer (Rust): Mistakes & Learnings

A record of my first attempt at producer-consumer in Rust, the bugs I hit, and what each one taught me about Rust vs C++.

---

## The headline observation

I came in with the algorithm clear from the C++ exercise — predicate disjunction, two condvars, the asymmetric `closed && empty` rule for `pop`. **Almost every bug I hit here was Rust syntax / idiom slippage, not concurrency.** That's encouraging for the algorithm and humbling for the language.

The few things I got right on the first attempt are worth naming so I don't take them for granted:
- The `while` predicate `!(space || closed)` (correctly: "wait while no space AND not closed").
- The `pop` asymmetry — bail with `None` only when `closed && empty`, not on `closed` alone. (This is *exactly* the Mistake #6 trap from the C++ writeup; reading that doc paid off.)
- Both `notify_one` partners (`pop` notifies `not_full`, `push` notifies `not_empty`).

---

## The journey at a glance

| Stage | Mistake | What it taught me |
|---|---|---|
| 1 | `std::move(item)` | Rust moves implicitly — by-value parameters already transfer ownership. No keyword needed. |
| 2 | `Ok()` | `Result<(), T>::Ok` carries a value; the unit type `()` is still a value, so `Ok(())`. |
| 3 | `if (cond) { Err(item) }` then fall-through | An `if` with no `else` evaluates to `()`. Either `return Err(item);` (with semicolon) or wrap the rest in `else { ... }` so the `if/else` is the function's tail expression. |
| 4 | `self.closed` | State lives behind the lock. `closed` is on `BufferInner`, accessed as `inner.closed` after `self.inner.lock()`. |
| 5 | `Ok(closed)` on bail | Wrong direction. Closed → bail with `Err(item)` and **return the unconsumed item** to the caller (so the producer can retry / drop it / log it — its choice). |
| 6 | `wait(inner).lock().unwrap()` | `Condvar::wait(guard)` returns the same `LockResult<MutexGuard>` shape that `Mutex::lock()` does. Just `.unwrap()` on the result — don't call `.lock()` on it as if it were a fresh `Mutex`. |
| 7 | `.empty()` on `VecDeque` | Rust naming convention is `is_empty` / `is_some` / `is_none`, not the C++ STL `empty()`. Compiler will suggest the right name. |
| 8 | `closed` bare in `pop`'s wait loop | Same as #4, but I missed it again in `pop` after fixing it in `push`. Lesson: when applying a fix, grep for the pattern across the whole function. |
| 9 | `inner.pop_front()` | `inner` is `MutexGuard<BufferInner<T>>`. `pop_front` lives on `inner.queue`, not the guard. (I'd written `inner.queue.push_back(item)` correctly two lines earlier — inconsistency caught me.) |
| 10 | Returning `item` after `.unwrap()` | `.unwrap()` extracted the `T` from `Option<T>`. Function return type is `Option<T>`. Have to re-wrap as `Some(item)`. |
| 11 | TSan ABI mismatch | The `RUSTFLAGS="-Z sanitizer=thread" cargo +nightly run` recipe in `docs/testing-for-bugs.md` doesn't work on recent nightlies — sanitized user code can't link against unsanitized prebuilt `std`. Need `-Z build-std --target x86_64-unknown-linux-gnu` so std is rebuilt with the same flag. |

---

## The recurring meta-lesson

Three of the bugs (#4, #7, #8) were "I fixed the pattern in one place and missed it in the other." The push/pop pair is a near-mirror, which makes it easy to think "I already did that" when I actually only did it on one side. **Mirror functions need mirror checklists.** Next time I'll do a sweep of *every* line of `pop` against the equivalent line of `push` before declaring done.

The other recurring class is C++ idioms leaking in:
- `std::move` (Rust uses implicit moves)
- `.empty()` (Rust uses `.is_empty()`)
- C-style parens around conditions: `if (cond) { ... }` and `while (cond) { ... }`. Rust accepts these but `unused_parens` warns. `cargo fix --allow-dirty` strips them.
- Bare flag access: `self.closed` reads like a C++ member access; in Rust the lock guard mediates everything inside.

---

## Aside: does Rust really require an explicit `while` around `wait`?

While writing the condvar version I noticed my Rust code has an explicit `while !pred { inner = cv.wait(inner).unwrap(); }`, but the C++ version on the same problem just writes `cv.wait(lock, pred)` with no visible loop. It *felt* like Rust was forcing me to do something C++ wasn't. That impression is wrong, and untangling it was useful.

### The shared truth

The loop has to exist somewhere because three things can wake you with the predicate still false:

1. **Spurious wakeups** — both POSIX condvars and Rust's `Condvar` are explicitly allowed to return from `wait()` without anyone calling notify.
2. **Stolen wakeups** — a fresh thread can grab the lock and consume the resource between "you were notified" and "you actually run."
3. **Broadcast excess** — `notify_all` wakes N threads when only K resources exist.

This is identical in C++ and Rust. The C++ writeup's slogan — *the condvar is just an attention mechanism, the shared state is the source of truth* — applies word-for-word in Rust.

### Both languages have BOTH API forms

C++:
```cpp
cv.wait(lock);                                  // bare — YOU write the while loop
cv.wait(lock, [&]{ return queue.size() > 0; }); // predicate overload — loop is INSIDE
```

Rust:
```rust
inner = cv.wait(inner).unwrap();                                  // bare
inner = cv.wait_while(inner, |s| !(s.has_space || s.closed)).unwrap(); // loop is INSIDE
```

`wait_while` is the Rust mirror of C++'s `wait(lock, pred)`. The condition closure is *"keep waiting while this is true"* — i.e. the negation of the C++ "wait until this is true" predicate. So my own code:

```rust
while !(inner.queue.len() < self.capacity || inner.closed) {
    inner = self.not_full.wait(inner).unwrap();
}
```

is exactly equivalent to:

```rust
inner = self.not_full
    .wait_while(inner, |s| !(s.queue.len() < self.capacity || s.closed))
    .unwrap();
```

### Why the cultural difference

- **C++** rewards the predicate overload because the explicit loop is verbose and easy to get wrong (forget `while`, capture the wrong variable, etc.). Stroustrup and most style guides recommend "always use the predicate overload."
- **Rust's** `wait` *returns the guard back to you*, which is unusual — it foregrounds the lock-handoff dance. The explicit loop reads naturally because you're already writing `inner = ...wait(inner)...` to thread the guard through. Hiding it inside `wait_while`'s closure works but feels less direct, so most Rust tutorials and templates (including mine) show the bare form.

The result: the loop is mandatory in both languages; only its *visibility* differs by convention.

### Sabotage experiment for next time

Rewrite my `push` and `pop` using `wait_while` instead of `wait` + manual loop, and see whether the resulting code reads better or worse to me. (Prediction: in `pop` the asymmetric bail check still wants to live outside the wait, so the explicit-loop version may actually feel cleaner because the post-wait reasoning is right next to it.)

---

## What I'd do before declaring "done" next time

The shape that worked here, in order:
1. **Write the algorithm in English first.** Predicate, asymmetry, who notifies whom. (I had this — from the C++ exercise.)
2. **Translate to Rust syntax slowly.** Each `let`, each `.unwrap()`, each `Some(_)` wrap.
3. **`cargo check` early.** Don't wait until you've written `pop` to see whether `push` compiles. The compiler is the fastest reviewer in the room.
4. **Mirror-sweep.** For symmetric pairs (push/pop, send/recv), check each line of one against the other.
5. **Stress + sanitizer.** One run is meaningless. The bar is 1 000+ runs and a clean TSan pass. (Both passed for this implementation.)

---

## Final results

- Cold run: `Produced=150 Consumed=150 Expected=150` ✓
- 1 000-run stress loop: 1 000 ok / 0 fail ✓
- ThreadSanitizer (with `-Z build-std`): clean, no warnings ✓

---

## Working `pop` (for future reference)

```rust
pub fn pop(&self) -> Option<T> {
    let mut inner = self.inner.lock().unwrap();
    while inner.queue.is_empty() && !inner.closed {
        inner = self.not_empty.wait(inner).unwrap();
    }
    if inner.closed && inner.queue.is_empty() {
        None
    } else {
        let item = inner.queue.pop_front().unwrap();
        self.not_full.notify_one();
        Some(item)
    }
}
```

The `.unwrap()` on `pop_front()` is intentional: it asserts my own invariant ("by the time we reach this branch, the queue must be non-empty") and crashes loudly if the predicate logic ever drifts.

---

## Next moves

- [x] Rewrite using `std::sync::mpsc::sync_channel(capacity)` — the "easy" Rust way. Compare LOC and where the complexity moved. *(Done — see Part 2 below.)*
- [ ] Try `crossbeam-channel` for true multi-consumer support (std `mpsc` is single-consumer only).
- [ ] Sabotage experiments to cement the lessons:
  - Change `while` to `if` around `wait` → run under stress, predict and observe failure.
  - Drop the `inner.queue.is_empty()` clause from the `pop` bail check → predict what gets lost on shutdown.
  - Change `notify_all` in `close()` to `notify_one` → predict the hang.

---

# Part 2: The mpsc variant (`src/bin/mpsc.rs`)

## The headline observation

Where the condvar version's bugs were about **algorithm translation** (C++ idioms leaking into Rust), the mpsc version's bugs split into two clean buckets:

1. **Rust surface syntax I still hadn't internalised** — ranges (`0...N` vs `0..N`), block-returns-value semicolons, match arm syntax, method-vs-free-function.
2. **The two pitfalls the template header explicitly warned about** — dropping `tx`, and sharing the `Receiver`.

The lesson is almost self-congratulatingly obvious in retrospect: **the pitfalls box at the top of a template is a cheat sheet. Read it, paraphrase it back to yourself, and only then start typing.** I didn't, and I paid for each pitfall individually.

---

## The journey at a glance

| # | Mistake | Fix | Category |
|---|---|---|---|
| 1 | `let (tx, rx) = mpsc::channel;` | `let (tx, rx) = sync_channel::<String>(BUFFER_SIZE);` — missing `()`, wrong path (needs import), and `channel` vs `sync_channel` for bounded | syntax |
| 2 | `for i in 0...NUM_PRODUCERS` | `0..NUM_PRODUCERS`. Rust: `..` exclusive, `..=` inclusive. `...` is a legacy syntax that was removed; no tripled dot means "range" anywhere in modern Rust. | syntax |
| 3 | Swapped `pid` / `i` in producer loops | Outer loop variable names the producer (`pid`), inner names the item index (`i`). I had them flipped, so every "producer 0" message was actually item 0 from every producer. | naming |
| 4 | Consumer block on main thread with shadowing: `let consumed = rx.recv().unwrap();` → `consumed.fetch_add(...)` | `consumed: String` (shadowed the `Arc<AtomicI32>`), so `fetch_add` was being called on a `String` — compile error. Also sidestepped the exercise entirely. | scope/pitfall |
| 5 | **Pitfall (A) forgotten**: original `tx` held in main for the entire run | `drop(tx);` immediately after spawning all producers. Otherwise `rx.recv()` never returns `Err` (sender count never reaches 0), and consumers deadlock waiting for a message that can never arrive. | pitfall |
| 6 | **Pitfall (B) forgotten**: `Receiver` is not `Clone` | Wrap in `Arc::new(Mutex::new(rx))`, give each consumer a clone. Sharing `Receiver` across threads always needs this. | pitfall |
| 7 | C++-style type declaration: `Arc<Mutex<Receiver<String>> recv_lock;` | Rust infers types; never declare-without-initialise. `let recv_lock = Arc::new(Mutex::new(rx));` | syntax |
| 8 | `Arc::clone(%recv_lock)` | `&recv_lock`. `%` is not a reference operator in any language I know — finger slip. | typo |
| 9 | `produced` moved into first spawned thread, no clone | Clone the `Arc` **before** every `thread::spawn`. See the Arc::clone deep-dive below. | ownership |
| 10 | Closure body used outer `recv_lock`, not the cloned `recv_lock_clone` | Always reference the cloned name inside the closure, not the original. When I rename inconsistently, Rust will either compile it wrong (capture the outer) or — after moves — refuse to compile. | shadowing |
| 11 | `recv()` called as a free function | `recv_handle.recv()` — `recv` is a method on `Receiver`, never a free function. | API |
| 12 | `let mut item = recv_handle.recv();` | `let item = ...;` — never reassigned, so `mut` is noise. Also harmful: makes me *think* I'll reassign later, which I shouldn't. | warning |
| 13 | `match item { Ok() => ..., Err() => ..., }` | `Ok(_)` / `Err(_)` or `Ok(s)` / `Err(e)` — the parens need a pattern, even if you don't care about the value. | syntax |
| 14 | Arms separated by `;` | Arms separated by `,`. Inside each arm, statements still use `;`. | syntax |
| 15 | `break` with no surrounding `loop` | Add `loop { ... }` around the recv+match body. A consumer pulls items until shutdown — it doesn't process one then exit. | structure |
| 16 | `std::hint::black_box(item)` inside `Ok(s) => {...}` | `match item` **moved** `item`. Inside `Ok(s)` the only valid binding is `s`. This is one of the most common "value used after move" mistakes in Rust. | ownership |
| 17 | Lock held across `recv()` blocking wait | Scope the lock tightly: `let item = { let rx = lock.lock().unwrap(); rx.recv() };`. Holding the guard across the blocking wait serialises consumers not just on dequeue but on *waiting* — defeats the whole point of Arc<Mutex<_>>. | concurrency |
| 18 | Semicolon inside the block: `rx.recv();` | Without the trailing `;`, `rx.recv()` is the block's value. With `;`, the block evaluates to `()`. Caught by the compiler: `expected Result, found ()`. | syntax |
| 19 | Missing `;` after `let item = { ... }` | A `let` is a statement and needs its terminating `;`, even when the RHS is a block. Without it, the parser consumes the following `match` as part of the expression. | syntax |
| 20 | `h.join.unwrap();` | `h.join().unwrap();` — `join` is a method, not a field. | typo |
| 21 | No `JoinHandle`s collected | Without `.join()`, main races past the spawn loops and hits the asserts before any consumer has run. `assert_eq!(c, expected)` fires with `c=0`. Collect `Vec<JoinHandle<()>>` for producers and consumers, join both before the asserts. | structure |
| 22 | Missing producer jitter `thread::sleep(Duration::from_millis((pid as u64) % 3))` | Without it, producers never block on a full buffer, so `BUFFER_SIZE=8` is never reached and the backpressure path of `sync_channel` isn't exercised. The program still passes, but tells you nothing about the bounded-buffer behaviour. | template |

---

## Deep dive: `Arc::clone` and the per-thread clone pattern

This is the single idiom I got wrong the most times, so it's worth slowing down.

### What `Arc<T>` actually is

`Arc<T>` = **A**tomically **R**eference-**C**ounted pointer. It's a heap-allocated pair `(refcount, T)`. Every `Arc<T>` is a smart pointer to that block:

- Cloning the `Arc` → atomic increment of the refcount. Cheap, O(1), no deep copy of `T`.
- Dropping the `Arc` → atomic decrement. When the count hits 0, the inner `T` is dropped.
- Because the refcount is atomic, multiple threads can hold clones of the same `Arc<T>` safely.

So `Arc<T>` is the "I want shared, read-only, immutable access to the same `T` from multiple threads" tool. For *mutable* sharing, the inner `T` has to provide its own thread-safety: `Arc<AtomicI32>`, `Arc<Mutex<U>>`, `Arc<RwLock<U>>`. `Arc` itself doesn't give you interior mutability.

### The two-axis mental model: Arc = spatial sharing, Mutex = temporal exclusion

The most useful reframe I landed on during this exercise:

- **`Arc<T>` → spatial sharing.** Multiple threads can each hold a handle pointing at the same heap location.
- **`Mutex<T>` → temporal exclusion.** Only one of those handles can actually *dereference into* the `T` at any given moment; the others have to wait their turn.

Both sentences describe the same `Arc<Mutex<T>>` value — they're two different *dimensions* of how access works. The Arc spreads reachability across threads; the Mutex serialises actual access over time.

### Arc<T> never hands out `&mut T` — interior mutability is the escape hatch

This is the key rule. `Arc<T>` methods only ever give you `&T` (shared / immutable reference). Never `&mut T`. So a bare `Arc<i32>` is genuinely read-only — you cannot mutate the `i32`, ever, no matter how hard you try.

To mutate through an `Arc`, the wrapped type has to offer **interior mutability** — mutation through a shared `&self`:

| Inner type | Call that mutates | What it gives you |
|---|---|---|
| `Mutex<T>` | `.lock(&self)` | `MutexGuard<T>` that derefs to `&mut T` |
| `RwLock<T>` | `.read(&self)` / `.write(&self)` | `&T` or `&mut T` |
| `AtomicI32` | `.fetch_add(&self, ...)`, etc. | lock-free atomic op through `&self` |
| `RefCell<T>` | `.borrow_mut(&self)` | `RefMut<T>` — but single-threaded only, so not usable under `Arc` across threads |

So when you see `Arc<Mutex<T>>` in the wild, the two halves are doing separate jobs: `Arc` makes the `Mutex<T>` reachable from many threads, `Mutex<T>` makes the `T` mutable through a shared reference. Swap `Mutex` for `AtomicI32` and you get the same story for a simpler inner type — hence `Arc<AtomicI32>` for the counters in this code, with no `Mutex` needed because `AtomicI32` already provides its own exclusion.

A corollary that catches people out: **when the `MutexGuard` drops, you don't have "shared access" to the inner `T`. You have no access at all.** To touch the `T` again you must call `.lock()` again, possibly blocking. "Unlocked" doesn't mean "everyone can read it now"; it means "no one currently holds a reference to it, and whoever grabs the lock next will get exclusive access." See the pitfall (B) deep-dive below for the timeline.

### Why you need to clone per `thread::spawn`

`thread::spawn(move || { ... })` **moves** every captured variable **into** the closure. Once a variable is moved, the outer scope loses access to it. So this:

```rust
let produced = Arc::new(AtomicI32::new(0));
for pid in 0..NUM_PRODUCERS {
    thread::spawn(move || {
        produced.fetch_add(1, Ordering::SeqCst);  // ← captures `produced` by move
    });
}
```

…fails on the **second** iteration. Iteration 0 moved `produced` into its closure; iteration 1 has no `produced` in scope to move. Error:

```
error[E0382]: use of moved value: `produced`
```

The fix is to hand each thread its own `Arc` clone, which all point to the same underlying `AtomicI32`:

```rust
for pid in 0..NUM_PRODUCERS {
    let produced = Arc::clone(&produced);  // new Arc, same inner AtomicI32
    thread::spawn(move || {
        produced.fetch_add(1, Ordering::SeqCst);
    });
}
```

Now every iteration creates its own `Arc` clone, which the `move` closure consumes. The outer `produced` stays alive because the clones are independent `Arc` instances — only the refcount changes.

### `Arc::clone(&x)` vs `x.clone()` — they behave the same, but…

Both call the same `Clone` impl. Idiomatic Rust prefers `Arc::clone(&x)`:

- It makes the cost visible at the call site. `Arc::clone` is obviously "refcount bump, O(1)". `x.clone()` looks like it could be an expensive deep-clone of the inner type (which it absolutely is if `T: Clone` and you accidentally call the inner clone).
- If `T: Clone`, calling `x.clone()` on an `Arc<T>` still produces an `Arc<T>` (because `Arc: Clone`), but a reader has to know that to be sure. `Arc::clone(&x)` cuts the ambiguity.

### The shadowing idiom

A very common pattern you'll see in Rust tutorials and real code:

```rust
for _ in 0..N {
    let produced = Arc::clone(&produced);  // shadows the outer name inside the loop body
    thread::spawn(move || {
        produced.fetch_add(1, Ordering::SeqCst);  // this is the clone, not the original
    });
}
```

The inner `let produced = ...` shadows the outer binding only inside this iteration. The original outer `produced` is untouched, because `Arc::clone(&produced)` borrows it, never moves it. I find this pattern slightly confusing to read ("which `produced` is this?") so I used explicit `produced_clone` names in my code. Either is fine; consistency within a codebase matters more than which you pick.

### The "I cloned but forgot to use the clone" bug

This one bit me on line 102 of an intermediate revision:

```rust
let recv_lock_clone = Arc::clone(&recv_lock);  // cloned correctly
thread::spawn(move || {
    let guard = recv_lock.lock().unwrap();     // ← using the OUTER, un-cloned Arc
    ...
});
```

What happens depends on whether `recv_lock` has already been moved. If it has, the closure can't capture it → compile error. If it hasn't (first iteration), the closure captures `recv_lock` by move, and `recv_lock_clone` is *unused* — subsequent iterations fail. Either way you lose. The rule: **the clone and the usage must share a name**. That's what the shadowing idiom enforces automatically.

### The three Arcs in the mpsc solution and why each one exists

| Arc | Why |
|---|---|
| `Arc<AtomicI32>` — `produced` | Shared *mutable* counter. `AtomicI32` provides the mutability (lock-free); `Arc` provides the sharing across threads. |
| `Arc<AtomicI32>` — `consumed` | Same reason, consumer side. |
| `Arc<Mutex<Receiver<String>>>` — `recv_lock` | `Receiver` is not `Clone`, and `Mutex::lock()` only gives access to its contents to one thread at a time. `Arc` lets both consumer threads hold a handle; `Mutex` enforces "only one of them calls `recv` at any moment." |

The bounded `SyncSender` (returned by `sync_channel`) is special: it's already `Clone`, so you do *not* need `Arc` around it. You just `tx.clone()` once per producer thread, and main's original `tx` is dropped to signal shutdown. That's what makes the sender side so clean. (The reason `SyncSender` can be `Clone` while `Receiver` can't is the next section's story.)

---

## Deep dive: channel anatomy — `tx` and `rx` are handles, not channels

This confused me for longer than it should have. I kept thinking of `tx` and `rx` as "the sending channel" and "the receiving channel." Neither is a channel. They're both handles to the *same* underlying channel.

### What `sync_channel(8)` actually allocates

```rust
let (tx, rx) = sync_channel::<String>(8);
```

Behind the scenes Rust allocates a **single** data structure on the heap — the actual channel — containing roughly:

```
[ bounded buffer (Vec<String>, cap=8) ]
[ internal lock + two condvars (or equivalent) ]
[ sender refcount ]
[ receiver state ]
```

Then it hands you **two different smart-pointer-like handles** that both point at that one structure:

- `tx: SyncSender<String>` — the "drop a letter into the mailbox" handle.
- `rx: Receiver<String>` — the "take a letter out of the mailbox" handle.

The mailbox is the channel. `tx` and `rx` are the slot and the key.

### Two handle types with deliberately different rules

| | `SyncSender<T>` (`tx`) | `Receiver<T>` (`rx`) |
|---|---|---|
| `Clone`? | **Yes** — multiple producers allowed. | **No** — single consumer by design. |
| `Send`? | Yes (move to another thread) | Yes |
| `Sync`? | Yes (share `&tx` across threads) | **No** — cannot be shared via `&`, which is *why* you need `Mutex` to hand it to multiple consumers. |
| Role | Pushes into the buffer; blocks if full (since `sync_channel` is bounded). | Pops from the buffer; blocks if empty. |
| Shutdown effect | When *all* `SyncSender`s are dropped, `rx.recv()` will eventually return `Err`. | When `Receiver` is dropped, further `send`s return `Err(SendError)`. |

The asymmetry is the "multi-producer single-consumer" part of `mpsc`.

### Why `tx.clone()` is essentially an `Arc::clone`

`SyncSender` is itself implemented using `Arc`-like refcounting over the internal channel state. Every clone is another handle; every drop decrements a sender counter. When the counter hits zero, the internal channel transitions to "closed," and `rx.recv()` starts returning `Err(RecvError)` once the buffer drains. That is literally the mechanism behind pitfall (A): `drop(tx)` in main is you decrementing the sender refcount to its final value.

So the Arc mental model from the previous section transfers directly:

- `Arc::clone(&produced)` bumps a refcount on the shared `AtomicI32`.
- `tx.clone()` bumps a refcount on the shared channel (on the sender side specifically).

The mechanics are the same. `SyncSender` just wraps it in a nicer API and ties the refcount to channel-close semantics.

### Why `Receiver` specifically needs `Arc<Mutex<_>>` to be shared

`Receiver<T>` is `Send` but **not `Sync`**. That means:

- You *can* move a `Receiver` to another thread (Send).
- You *cannot* share `&Receiver` between threads simultaneously (not Sync) — the internal implementation uses operations that would be unsound if two threads called `recv` on the same `Receiver` at the same time.

So if two consumer threads want to pull from the same channel, the only safe way is to put the single `Receiver` behind something that enforces "only one thread holds it at a time." That's exactly what `Mutex<Receiver<T>>` does, and `Arc<Mutex<Receiver<T>>>` lets both threads *reach* that mutex.

This is also why `SyncSender` *doesn't* need an `Arc<Mutex<_>>` wrapper — it's `Sync`, so sharing `&tx` between threads is already sound. You just `clone()` once per thread and each thread owns its own handle outright.

### The picture (3 producers, 2 consumers)

```
            ┌─ tx_clone (prod 0) ───┐
            ├─ tx_clone (prod 1) ───┤
            ├─ tx_clone (prod 2) ───┤       ┌─── Arc<Mutex<Receiver>> ───┐
 (main drops┤  the original tx   ──┤       │                             │
  tx before └───────────────────────┤       │   cons 0      cons 1       │
  consumers)                        │       └─────────────────────────────┘
                                    │                     │
                                    ▼                     ▼
                  ┌──────────────────────────────────────────────────┐
                  │  THE CHANNEL (heap-allocated, single structure)  │
                  │  - bounded buffer (cap 8)                        │
                  │  - internal mutex + condvars                     │
                  │  - sender refcount (here: 3 after main drops)    │
                  └──────────────────────────────────────────────────┘
```

The boxes on top aren't separate channels — they're handles pointing at the one structure on the bottom. `send(item)` on *any* `tx_clone` pushes into the same buffer; `recv()` on `rx` (under the mutex) pops from the same buffer.

### So what's actually "shared" and what's not

- The **channel struct on the heap**: shared across all five threads. Refcount + internal lock prevent races on its own state.
- A `SyncSender` **handle** (`tx_clone`): owned by one thread at a time. Cheap to clone when another thread needs to send.
- The `Receiver` **handle** (`rx`): owned by `Arc<Mutex<_>>` — the `Arc` lets multiple threads reach the `Mutex`, the `Mutex` serialises access to the single `Receiver` handle.

### The tight definition

> `tx` and `rx` are two different kinds of smart pointer into one underlying channel object. The channel is the *thing*; `tx` and `rx` are *views* of that thing with opposite capabilities (push vs pop) and different sharing rules (clone freely vs don't).

Once this clicks, `drop(tx)` stops looking like magic. It's the same language as dropping an `Arc`: "this view is no longer holding the channel alive on the sender side." When the last sender-view drops, the receiving side can finally see that no more messages are coming.

---

## Deep dive: pitfall (A) — `drop(tx)` before waiting

Channels in `std::sync::mpsc` close on a reference-count principle. `rx.recv()` returns `Err(RecvError)` **only** when *all* `SyncSender` clones have been dropped. One sender still alive anywhere in the program → `recv()` keeps blocking, because it has no way to know no more messages are coming.

The bug shape:
```rust
let (tx, rx) = sync_channel::<String>(8);
for pid in 0..N_PRODUCERS {
    let tx_clone = tx.clone();
    thread::spawn(move || { /* produce, drop tx_clone on thread exit */ });
}
// ← tx is STILL alive here, held by main()
while let Ok(msg) = rx.recv() { /* hangs forever after producers finish */ }
```

Each producer thread drops its `tx_clone` when it exits. That would be enough to close the channel — except main is still holding the *original* `tx`. Refcount never reaches 0; consumers wait forever.

Fix: `drop(tx);` immediately after the spawn loop, **before** consumers start waiting. The thinking process I'd use next time:

1. After every `sync_channel` creation, ask: "what path does each `SyncSender` take to drop?"
2. Count the alive senders at the moment consumers are waiting. If > (# producer threads), something's still held somewhere.
3. The original in main is the usual culprit.

This is the replacement for the condvar version's `close()` + `notify_all()`. Zero lines of shutdown code — `drop` does it. One of the big arguments for the channel abstraction.

---

## Deep dive: pitfall (B) — `Arc<Mutex<Receiver>>` and lock scope

`Receiver` doesn't implement `Clone`. The stdlib could have made it Clone (and `crossbeam-channel` does), but the standard-library choice was to keep std `mpsc` single-consumer. If you want multiple consumer threads you have to put it behind a mutex.

Once you do, there's a subtle footgun: lock scope.

**Wrong:**
```rust
let guard = recv_lock_clone.lock().unwrap();   // acquire
let item = guard.recv();                        // ← block here, still holding guard
match item { ... }
// guard drops at end of iteration
```

While consumer A is inside `recv()` waiting for a message, it holds the mutex. Consumer B calls `.lock()` and blocks — **not on the recv, but on acquiring the mutex**. You've now got two threads, both blocked, only one of which can actually make progress when a message arrives. That's a serialised consumer, but worse than the single-threaded main-only version because you've added lock contention on top.

**Right:**
```rust
let item = {
    let rx = recv_lock_clone.lock().unwrap();
    rx.recv()
};  // guard dropped HERE, immediately after recv returns
match item { ... }
```

The inner block binds the guard, calls `recv`, and the guard drops as soon as the block ends. Yes, the `recv` still happens under the lock — so `std::sync::mpsc` with `Arc<Mutex<_>>` is still effectively single-consumer — but now the lock is released before the match body runs, so the *other* consumer can step in for the next item the moment the first one wakes up.

The "in general" lesson: **lock scope = critical section**. The only lines inside the lock should be the ones that touch the shared state. Everything else (the match, the black_box, the fetch_add) touches thread-local data and belongs outside.

### Clarifying "when the guard drops" — no, it's not "shared access"

A subtlety I had to untangle: "the guard drops" does **not** mean the inner `T` becomes readable by everyone simultaneously. It means *no one* holds a reference to it, until someone calls `.lock()` again.

Timeline across two consumer threads sharing an `Arc<Mutex<Receiver<String>>>`:

```
Thread A                          Thread B
let rx = arc.lock().unwrap();     let rx = arc.lock().unwrap();   ← blocks on mutex
let item = rx.recv();             (still blocked)
drop(rx);  // guard released
                                   (wakes up, gets the guard)
                                   let item = rx.recv();
                                   drop(rx);
```

At every instant, *at most one* thread holds any reference into the `Receiver`. After `drop(rx)` in thread A, **A also no longer has access** — `rx` is gone from its scope too. If A wants to touch the `Receiver` again it must call `.lock()` again and possibly wait.

So the end-to-end picture, restated:

- **`Arc`** makes the `Mutex<Receiver<String>>` **reachable** from both consumer threads.
- **`Mutex`** guarantees that at any moment, at most one of those threads holds a `MutexGuard` into the `Receiver`.
- The `Receiver` itself is never "shared at the same time" — sharing happens *over time*, one thread at a time.

This is also why the writeup's honest comparison has to say: *NUM_CONSUMERS = 2, but effective consumer concurrency is 1*. The `crossbeam-channel` crate removes this restriction because its `Receiver` is `Clone` and `Sync`, so no mutex is needed — true simultaneous multi-consumer access.

---

## Deep dive: `match` consumes its scrutinee

The bug at line 106 of one revision:

```rust
match item {
    Ok(s) => {
        std::hint::black_box(item);  // ← error: use of moved value
        ...
    }
    Err(_) => break,
}
```

Mental model: `match item { ... }` takes `item` by value (moves it) unless you write `match &item { ... }`. The patterns in the arms destructure the moved value. Inside `Ok(s)`, the only remaining binding is `s` — the original `item` is gone.

The symmetry is clean once you hold the rule: **anything you want to use after the match, you must `ref`-borrow or re-bind out of the arm.** Here I just wanted to acknowledge the string, so `black_box(s)` was correct.

---

## The condvar-vs-mpsc complexity comparison

Now that both versions are working, the comparison the template header asks for has real numbers:

|  | Condvar version (`src/main.rs`) | mpsc version (`src/bin/mpsc.rs`) |
|---|---|---|
| **Queue mgmt LOC** | ~20 (struct, `push`, `pop`, two condvars, `close`) | 0 — it's inside `sync_channel` |
| **Steady-state wait logic** | explicit `while !pred { cv.wait() }` in both `push` and `pop` | implicit in `send` / `recv` |
| **Shutdown mechanism** | `close()` flips a flag + `notify_all` on both cvs | `drop(tx)` |
| **Shutdown LOC I wrote** | ~5 | 1 |
| **Consumer concurrency** | unbounded (each consumer holds its own guard briefly) | **1** (serialised by `Arc<Mutex<Receiver>>`) |
| **Where did the complexity go?** | was in the data structure | now in the *shape* (who holds which `tx` clone, when to drop) |

The mpsc version is shorter, but trades flexibility for convenience: you can't rebuild `sync_channel`'s policy (FIFO-only, exactly-once delivery, sender-count-based shutdown). The condvar version gives you a queue you can extend (priority, timeout, drop-oldest) without rewriting the waiting machinery.

---

## The recurring meta-lesson, part 2

The condvar version's meta-lesson was "mirror functions need mirror checklists." This version's is different:

**Read the pitfall list before you type.** I had a two-line comment block at the top of `mpsc.rs` that literally said "you will forget to drop tx" and "Receiver is not Clone." I hit both bugs. Not because the warnings were unclear, but because I started typing before paraphrasing them.

The fix isn't more self-discipline. It's a ritual: before touching the TODO block, write two or three sentences restating the pitfalls in my own words, at the keyboard. Reading alone is passive; restating is active.

---

## Final results

- Cold run: `Produced=150 Consumed=150 Expected=150` ✓
- 100-run stress loop: 100 ok / 0 fail ✓
- ThreadSanitizer (with `-Z build-std --target x86_64-unknown-linux-gnu`): 50/50 clean, no warnings ✓

---

## Working `mpsc.rs` structure (for future reference)

```rust
let (tx, rx) = sync_channel::<String>(BUFFER_SIZE);

// producers
let mut prod_handles = Vec::new();
for pid in 0..NUM_PRODUCERS {
    let tx_clone = tx.clone();
    let produced_clone = Arc::clone(&produced);
    prod_handles.push(thread::spawn(move || {
        for i in 0..ITEMS_PER_PRODUCER {
            let item = format!("P{}#{}", pid, i);
            tx_clone.send(item).unwrap();
            produced_clone.fetch_add(1, Ordering::SeqCst);
            thread::sleep(Duration::from_millis((pid as u64) % 3));
        }
    }));
}
drop(tx);  // pitfall (A): main's original sender must be dropped

// consumers
let recv_lock = Arc::new(Mutex::new(rx));
let mut cons_handles = Vec::new();
for _ in 0..NUM_CONSUMERS {
    let consumed_clone = Arc::clone(&consumed);
    let recv_lock_clone = Arc::clone(&recv_lock);
    cons_handles.push(thread::spawn(move || {
        loop {
            let item = {
                let rx = recv_lock_clone.lock().unwrap();
                rx.recv()
            };  // guard dropped here — pitfall (B) handled
            match item {
                Ok(s) => {
                    std::hint::black_box(s);
                    consumed_clone.fetch_add(1, Ordering::SeqCst);
                }
                Err(_) => break,
            }
        }
    }));
}

for h in prod_handles { h.join().unwrap(); }
for h in cons_handles { h.join().unwrap(); }
```

Three Arc clones, one drop, one mutex-scoped recv. Compare the LOC to the condvar version's `BufferInner` / `push` / `pop` / `close`.

---

## Next moves, take 2

- [ ] **`crossbeam-channel` version.** Same solution but `Receiver: Clone` means no `Arc<Mutex<_>>`, and `NUM_CONSUMERS = 2` becomes *real* 2-way concurrency. That should shave a few lines and remove the only "effectively serial" caveat in the current writeup.
- [ ] **Sabotage: remove `drop(tx)`.** Predict: consumers hang. Observe by letting the stress-loop time out.
- [ ] **Sabotage: hold the mutex across the match body.** Predict: throughput drops measurably (add a `println!` inside the match and time 1 000 runs with vs without the tight lock scope).
- [ ] **Sabotage: change `sync_channel(8)` to `sync_channel(0)` (rendezvous).** Predict: producers block on every send until a consumer is ready. Count how often the buffer is "full" with a counter before vs after.
