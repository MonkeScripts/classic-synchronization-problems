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

- [ ] Rewrite using `std::sync::mpsc::sync_channel(capacity)` — the "easy" Rust way. Compare LOC and where the complexity moved.
- [ ] Try `crossbeam-channel` for true multi-consumer support (std `mpsc` is single-consumer only).
- [ ] Sabotage experiments to cement the lessons:
  - Change `while` to `if` around `wait` → run under stress, predict and observe failure.
  - Drop the `inner.queue.is_empty()` clause from the `pop` bail check → predict what gets lost on shutdown.
  - Change `notify_all` in `close()` to `notify_one` → predict the hang.
