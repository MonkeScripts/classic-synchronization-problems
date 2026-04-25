# Barrier: Mistakes & Learnings

A record of implementing the reusable two-turnstile barrier in C++.

---

## Reference pattern: two-turnstile domino

This is the shape I settled on for `MyBarrier` in `main.cpp`. One mutex, two semaphores, two phases, domino release.

```
arrive_and_wait():
    // --- Phase 1: gather everyone ---
    lock mutex
    count++
    if count == N:
        turnstile2.acquire()   // close exit gate
        turnstile1.release()   // open entry gate
    unlock mutex

    turnstile1.acquire()       // enter
    turnstile1.release()       // domino in

    // --- Phase 2: leave everyone together ---
    lock mutex
    count--
    if count == 0:
        turnstile1.acquire()   // close entry gate (reset)
        turnstile2.release()   // open exit gate
    unlock mutex

    turnstile2.acquire()       // leave
    turnstile2.release()       // domino out
```

### Why each line is there

- **`turnstile2.acquire()` inside the lock in phase 1** — closes the exit gate *before* anyone starts leaving. Skip this and threads could race through phase 2 before the last arriver has even opened phase 1.
- **`turnstile1.release()` / `turnstile1.acquire()` pair in the domino** — only one token is released by the N-th arriver; each thread acquires it, then releases it again so the *next* thread can pass. This is the "domino" — one token walked through N threads.
- **`turnstile1.acquire()` inside the lock in phase 2** — drains the one leftover token so it doesn't leak into round R+1, where a thread at the entry domino could grab it and bypass the barrier. This is the part I almost forgot. Without it, reusability breaks on round 2.
- **Starting values**: `turnstile1 = 0` (entry closed), `turnstile2 = 1` (exit open — only matters for the symmetry of the first round's "close exit" step; the N-th arriver acquires it to 0).

### State cycle across one round

Let `(count, t1, t2)` be the full state. The cycle is `(0, 0, 1) → (0, 0, 1)`:

| Step | count | t1 | t2 |
|---|---|---|---|
| Start of round | 0 | 0 | 1 |
| After N-th thread's phase-1 lock block | N | 1 | 0 |
| After first domino (net zero effect) | N | 1 | 0 |
| After last-to-decrement phase-2 lock block | 0 | 0 | 1 |
| After second domino (net zero effect) | 0 | 0 | 1 |

End state = start state. No explicit reset needed beyond the two in-lock "close gate" ops.

---

## Contrast: preloaded turnstile (slide 22)

The `MyBarrierPreloaded` variant in the same file does `release(expected)` once and each thread acquires exactly once — no domino. Needs `counting_semaphore<N_THREADS>` (max ≥ N) because `release(N)` on a `counting_semaphore<1>` is UB.

Reusability is "free" there: N tokens put in, N tokens taken out, semaphore ends at 0 on its own. No drain line needed. The domino version needs the explicit `turnstile1.acquire()` drain in phase 2 precisely because release(1) leaves a leftover token the domino can't consume on its own.

---

## Contrast: mutex + condition_variable + generation counter

The `MyBarrierCond` variant in the same file ditches semaphores entirely:

```cpp
void arrive_and_wait() {
    std::unique_lock<std::mutex> lock(mu_);
    const std::uint64_t gen = generation_;
    ++count_;
    if (count_ == expected_) {
        count_ = 0;
        ++generation_;
        cv_.notify_all();
        return;
    }
    cv_.wait(lock, [this, gen] { return gen != generation_; });
}
```

Key ideas:

- **Generation counter, not a boolean flag.** A `bool ready_` would race: thread A from round R sees `ready_ == true`, exits, but its predicate can't tell that from round R+1's `ready_ == true`. The monotonic `gen` snapshot at entry uniquely identifies *this* thread's round, so when `gen != generation_`, the barrier has tripped *for me*.
- **Predicate-loop in `cv.wait(lock, pred)`** handles spurious wakeups for free. The two-arg `wait` is just a `while(!pred()) cv.wait(lock);` — same shape as the Go `for gen == b.generation { cv.Wait() }` loop.
- **`uint64_t`** for generation — overflow is theoretical (would take ~600 years at 1 GHz of barrier trips), but it's the right type for "monotonic counter".

### When to reach for which variant

| Variant | Pros | Cons |
|---|---|---|
| Domino (slide 20–21) | Works with `counting_semaphore<1>` (binary). | Needs the line-58 drain. Easy to get wrong. |
| Preloaded (slide 22) | Cleanest semaphore version. Reusable for free. | Requires `counting_semaphore<N>` (or higher). |
| Cond + generation | No semaphore needed at all. Predicate-loop kills spurious-wakeup class of bugs. | One extra `notify_all()` cost; CV's are slower than semaphores under heavy contention. |

In practice the cond version is what I'd write in production. The semaphore versions are pedagogical — they make the "why is this reusable" question concrete in a way the cond version handwaves through "oh, just bump the generation".

---

<!-- Add stages below as you hit and understand new bugs. -->
