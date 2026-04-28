# FIFO Semaphore (C++): Mistakes & Learnings

A record of implementing the FIFO semaphore in C++20 with two strategies — **ticket-queue + condvar** (`main.cpp`, mirrors lecture FIFOSemaphore2 + Task 1) and **queue of per-waiter binary semaphores** (`queue.cpp`, mirrors lecture FIFOSemaphore5) — the bugs along the way, and the conceptual model that made the lost-wakeup story click.

---

## The headline observation

A FIFO semaphore is `count + queue-of-waiters`. The two strategies differ in **how the queue is materialized**:

- **Ticket queue:** the queue is *implicit*. Atomic `next_ticket` + atomic `now_serving` give every waiter a number. A waiter proceeds when the counter reaches their ticket. Either spin (cheap, burns CPU) or park on a shared cv (cheap when contended, but you have to be careful about lost wakeups).
- **Queue of semaphores:** the queue is *explicit*. Each waiter has their own `binary_semaphore`; the queue stores them. `release()` pops the front and signals exactly that one. No shared wait set, no thundering herd, **structurally immune to lost wakeups** (binary_semaphore has memory; cv does not).

Both are FIFO by construction. The cv version is slightly cheaper per call (no allocation), but the queue version is harder to get wrong and scales better under heavy contention.

---

## Strategy 1: ticket queue + condvar (`main.cpp`)

The protocol:

```cpp
void acquire() {
    std::unique_lock lk{mut_};
    auto my_ticket = next_ticket.fetch_add(1);
    cv.wait(lk, [my_ticket, this] { return my_ticket <= now_serving; });
}

void release() {
    {
        std::scoped_lock lk{mut_};
        ++now_serving;
    }
    cv.notify_all();
}
```

### Mistake 1: lambda missing `return`

```cpp
cv.wait(lk, [current_ticket, this] {
    current_ticket >= now_serving;     // ❌ value computed and discarded
});
```

`cv.wait`'s predicate must return something convertible to `bool`. Without `return`, the lambda implicitly returns `void`. The compiler reports "could not convert from `void` to `bool`" inside cv's `while (!__p())` template, plus a `-Wunused-value` warning on the comparison itself.

### Mistake 2: predicate direction (twice)

First attempt: `return current_ticket >= now_serving;`. With `next_ticket{1}` and `now_serving{0}`, ticket 1 evaluates `1 >= 0 → true` immediately and falls through. Every acquire is a no-op.

Second attempt: `return current_ticket < now_serving;`. Now the equality case is excluded — when the counter just reaches your ticket, you should proceed, but `1 < 1 → false` keeps you waiting. Every thread hangs forever.

The right form is `<=`:

```cpp
return current_ticket <= now_serving;     // ✅ proceed when the counter has REACHED my ticket
```

The mental model: `now_serving` is "the highest ticket cleared so far." You proceed when the counter has caught up to (or passed) your ticket.

### Mistake 3: forgot to initialize `now_serving` from `initial_count`

```cpp
explicit FifoSemaphore(std::ptrdiff_t initial_count) {
    (void)initial_count;     // ❌ ignored
}

private:
    std::atomic<std::ptrdiff_t> now_serving;     // ❌ uninitialized
    std::atomic<std::ptrdiff_t> next_ticket{1};
```

The lecture's pattern is:

```cpp
explicit FifoSemaphore(std::ptrdiff_t initial_count)
    : now_serving{initial_count}, next_ticket{1} {}
```

These three pieces only make sense as a triple:
1. `next_ticket = 1`
2. `now_serving = initial_count`
3. predicate `my_ticket <= now_serving`

Ticket 1 satisfies the predicate iff `initial_count >= 1`; ticket 2 iff `initial_count >= 2`; …; ticket `initial_count` iff true; ticket `initial_count + 1` waits. So the **first `initial_count` calls to `acquire()` walk through without blocking**, which is exactly what a counting semaphore initialized to `N` should do.

I tried a variant — `next_ticket{initial_count + 1}, now_serving{initial_count}` — that *happened* to work for `INITIAL_COUNT = 0` (because the +1 cancels with the 0). For any other `initial_count`, the first ticket handed out is `initial_count + 1`, which never satisfies `<= initial_count` → no permits get pre-served. A `FifoSemaphore(2)` would behave like a `FifoSemaphore(0)`.

> **Lesson:** init values, predicate direction, and ticket numbering are three sides of the same design choice. Pick one canonical pairing and don't drift from it.

### Mistake 4 (subtle): lost wakeup in `release()`

```cpp
void release() {
    now_serving.fetch_add(1);   // ❌ mutated outside the lock
    cv.notify_all();
}
```

This is the deepest learning of this exercise. **Atomicity of the value is not the issue.** The issue is that `cv.notify_all()` has no memory — it only wakes threads currently parked on the cv. If you notify when nobody's parked, the notification just vaporizes.

The race window:

```
T (acquirer)                       R (releaser, no lock)
------------                       ----------------------
lock(mut_)
pred() → false
                                   now_serving.fetch_add(1)
                                   cv.notify_all()        ← WAKES NOBODY
                                                            (T not parked yet)
[cv.wait:
   unlock(mut_)
   park on cv]                                            ← T parks AFTER
                                                            the notify, misses it
... hangs forever
```

The `cv.wait`'s "release lock atomically with parking" guarantee is *only* atomic with respect to **other holders of the same mutex**. If R doesn't take the lock, R can race past the unlock-and-park.

Fix: lock briefly during the mutation. Notify can happen outside the lock; what matters is that the *mutation* is serialized with the predicate evaluation:

```cpp
void release() {
    {
        std::scoped_lock lk{mut_};
        ++now_serving;
    }
    cv.notify_all();
}
```

This is the same pattern that catches new producer-consumer programmers — "wait under a predicate, but mutate the predicate state under the same lock."

### Aside: `notify_all` vs `notify_one`

With a strict ticket queue, **at most one waiter** can have a satisfied predicate after a single release — the one whose ticket equals the new `now_serving`. Everyone else stays stuck with `my_ticket > now_serving`. So `notify_one()` is correct *and* avoids the thundering herd of N-1 waiters waking up only to immediately re-park.

`notify_all()` is the safe-by-default choice when you're not 100% sure which waiter is the right one. Per-ticket Notifies (one cv per waiter) get you `notify_one`'s precision, but at that point you're already drifting toward Strategy 2.

---

## Strategy 2: queue of per-waiter binary semaphores (`queue.cpp`)

The protocol:

```cpp
struct Waiter {
    std::binary_semaphore sem{0};
};

void acquire() {
    auto waiter = std::make_shared<Waiter>();
    {
        std::scoped_lock lk{mut_};
        if (count_ > 0) {
            --count_;
            return;          // permit available, take it without queueing
        }
        waiters_.push(waiter);
    }
    waiter->sem.acquire();   // park OUTSIDE the lock
}

void release() {
    std::shared_ptr<Waiter> waiter;
    {
        std::scoped_lock lk{mut_};
        if (waiters_.empty()) {
            ++count_;
            return;          // no waiter, permit becomes a free one
        }
        waiter = waiters_.front();
        waiters_.pop();      // permit transfers DIRECTLY to this waiter
    }
    waiter->sem.release();
}
```

### Mistake 5: `elapsed.count_()` from a clobbering rename

```cpp
std::chrono::duration_cast<std::chrono::milliseconds>(elapsed).count_()    // ❌
                                                              .count()    // ✅
```

I renamed `count` → `count_` (to distinguish the field from the method) and the search-replace ate the chrono duration's `.count()` method too. Compiler catches it: `'duration<...>' has no member named 'count_'`.

> **Lesson:** when renaming, prefer scoped renames (rename within an identifier-aware tool, not a text-level substitution) or use a unique name that avoids collisions with stdlib method names.

### Mistake 6: `release()` always pops, even on an empty queue → UB

```cpp
void release() {
    std::shared_ptr<Waiter> waiter;
    {
        std::scoped_lock lk{mut_};
        ++count_;
        waiter = waiters_.front();   // ❌ UB if waiters_ is empty
        waiters_.pop();              // ❌ UB if waiters_ is empty
    }
    waiter->sem.release();           // ❌ deref of garbage
}
```

`std::queue::front()` on an empty queue is undefined behavior — you get a garbage reference, and the `shared_ptr<Waiter> waiter` ends up holding garbage. Then `waiter->sem.release()` segfaults (or worse, doesn't, and you only notice in production).

The compiler can't catch this — front() on an empty queue is a *runtime* contract violation, not a type error.

### Mistake 7: `release()` increments `count_` even when waking a waiter

The same `release()` had the deeper logic bug: when a waiter is queued, the released permit goes **directly to that waiter** (you signal their semaphore). `count_` should not change. My version did `++count_` unconditionally *and* popped a waiter when one existed — emitting two permits per release.

Trace with `INITIAL_COUNT=0`, 16 threads, harness's 1 + 16 releases:

| step | action | count_ | waiters_ |
|---|---|---|---|
| start | | 0 | [] |
| 16 acquires | all queue (count_ was 0) | 0 | [T0..T15] |
| main release | count_++, pop T0, wake T0 | 1 | [T1..T15] |
| T0 release | count_++, pop T1, wake T1 | 2 | [T2..T15] |
| … | | | |
| T14 release | count_++, pop T15 | 16 | [] |
| T15 release | count_++, **front() on []** → UB | 17 | crash |

After 17 releases vs 16 acquires you should end with `count_ = 1`. With my code, count_ ended at 17 and the program segfaulted on the last release.

The fix is the **two-branch invariant**: a release does *one* of the following, never both:

> Either **increment `count_`** (no waiter is queued, so the permit becomes a free one)
> Or **transfer the permit to a queued waiter** (by signaling their semaphore)

Captured in code as:

```cpp
if (waiters_.empty()) {
    ++count_;
    return;
}
waiter = waiters_.front();
waiters_.pop();
```

> **Lesson:** when designing a primitive, write the invariant down first. "Each release emits exactly one permit, into either count_ or a waiter, never both." Code that violates this is wrong by construction.

---

## Why Strategy 2 is structurally lost-wakeup-immune

This is the conceptual difference between the two strategies, and worth naming explicitly.

| | cv | binary_semaphore |
|---|---|---|
| Memory of past notifications | **No** — notify into an empty cv vaporizes | **Yes** — release before acquire just sets the flag; next acquire returns immediately |
| Wake-up window | release must hold the same lock as the waiter, OR you have a lost-wakeup race | release can fire at any time, even before the waiter parks |
| Wait set | shared (one cv, many waiters) | private (one semaphore per waiter) |

In Strategy 2, by the time the releaser pops a waiter and calls `sem.release()`, that waiter is *already in the queue* (added under the lock). The waiter will park on `sem.acquire()` very shortly, but even if it hasn't yet — even if `sem.release()` fires *before* the waiter reaches `sem.acquire()` — the binary_semaphore stores the permit. The waiter gets it on its eventual call.

In Strategy 1, by contrast, the cv has no permit storage. If the notification fires between the predicate-check and the park, it's lost.

The price: one heap allocation per queued waiter (the `make_shared<Waiter>`). Lecture aside FIFOSemaphore7 reclaims it via an intrusive linked list (Waiters allocated on the acquirer's stack), at the cost of portability — you have to reason about object lifetimes around `release()` waking a Waiter that immediately exits its function.

---

## Summary: the C++ FIFO-semaphore muscle memory

### Ticket queue
1. **`next_ticket = 1`, `now_serving = initial_count`, predicate `my_ticket <= now_serving`.** All three pieces line up only when kept together. Drifting any one of them silently breaks `initial_count > 0`.
2. **Predicate direction matters: `<=`, not `<` and not `>=`.** `now_serving` is "highest ticket cleared so far"; you proceed when it has *reached* your ticket.
3. **Predicate must `return` something.** A bare expression statement implicitly returns `void`; cv.wait's `while (!__p())` then can't apply `!`.

### Lost wakeup (the deep one)
4. **`cv.notify_all` has no memory.** Notifying into an empty cv vaporizes the signal. Atomics on the predicate state don't help — the issue is *timing*, not value visibility.
5. **`cv.wait`'s unlock-and-park is atomic only w.r.t. other holders of the same mutex.** A mutator that doesn't take the lock can race past it. Always lock briefly during the mutation, even if the value itself is atomic.
6. **`notify_one` vs `notify_all`** — with a strict ticket queue, only one waiter's predicate is satisfied per release; `notify_one` is precise and avoids the thundering herd. Use it when you can prove there's only one viable waiter.

### Queue strategy
7. **`std::queue::front()` on an empty queue is UB.** Always check `empty()` before calling `front()`/`pop()`. The compiler won't catch this.
8. **Two-branch invariant for release: increment count OR transfer to a waiter, never both.** A release emits exactly one permit; the branches are mutually exclusive.
9. **Park on the per-waiter semaphore OUTSIDE the lock.** Holding the lock while parking would deadlock the next releaser.
10. **`shared_ptr<Waiter>` because the Waiter outlives the acquirer's stack frame** — it sits in the queue while the acquirer is parked. Stack-allocated alternatives (intrusive list) work but are non-portable.

### Cross-cutting
11. **`binary_semaphore` has memory; `cv` does not.** This is the structural reason Strategy 2 can't lose wakeups. Worth internalizing before designing your own primitives.
12. **When renaming, beware of name collisions with stdlib method names.** A blanket text-substitution of `count` → `count_` ate `chrono::duration::count()`. Use scoped/identifier-aware refactoring, or pick a name that won't collide.
