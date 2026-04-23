# Producer-Consumer: Mistakes & Learnings

A record of my first real attempt at producer-consumer in C++, the bugs I hit, and the concepts that clicked along the way.

---

## The journey at a glance

| Stage | Mistake | What it taught me |
|---|---|---|
| 1 | "If buffer is full, just return" | Polling vs blocking; why condvars exist |
| 2 | Inverted predicate (`wait until closed_`) | What `wait(lk, pred)` actually means |
| 3 | Empty lambda capture `[]` | Lambdas in member functions don't auto-capture `this` |
| 4 | Pushing after waking from closure | Why you must re-check state after `wait` returns |
| 5 | `return false` from a function returning `optional<T>` | `std::nullopt` is the right idiom |
| 6 | `pop` gave up on close even with items left | "Closed" ≠ "closed AND empty" |

Each one follows below with the full thinking.

After Part 1 landed green I reimplemented the *same interface* using two counting
semaphores instead of one mutex + two condvars. New primitive → new mistakes.
See **Part 2** further down for stages 7–10.

---

## Mistake 1: "Just return if the buffer is full"

### What I wrote
```cpp
if (buffer is full) return;
// caller retries in a loop outside
```

### Why it's wrong (sort of)
This *works*, but it's **busy-waiting**. The producer burns CPU cycles doing:
`lock → check full → unlock → retry → lock → check full → unlock → ...`

Two problems:
1. **Wasted CPU** — the producer isn't making progress but is consuming a core.
2. **Worse: the producer fights the consumer for the lock.** Every lock the producer grabs to check-and-fail is a lock the consumer needs to grab to actually pop an item. The faster the producer spins, the more it starves the consumer — the very thread that could free up space.

### The sentence that named the primitive
I wrote: *"When the buffer is full, I want the producer to wait for space and not block the consumer."*

- "wait" → block, don't spin
- "for space" → a specific predicate, not a generic signal
- "not block the consumer" → must release the mutex while waiting

That third point is the paradox condition variables exist to solve:
> **The producer must release the mutex AND go to sleep in one atomic step**, and re-acquire the mutex before waking. Otherwise there's a gap where a consumer could signal "space!" in between the producer's release and sleep — and the producer sleeps through the signal forever. This is called a **lost wakeup**.

That one-atomic-step operation is exactly what `std::condition_variable::wait(lock)` gives you.

---

## Mistake 2: Inverted predicate

### What I wrote
```cpp
not_full_.wait(lk, [] { return closed_; });
```

### What I thought it meant
"Wait when closed."

### What it actually means
`wait(lk, pred)` blocks **while the predicate is false** and wakes **when it becomes true**. Translating back: *"Block me until `closed_` is true."*

So my producer would sleep forever on a healthy buffer and only wake up after shutdown — to try pushing into a closed buffer. Exactly backwards.

### The fix — derived from the English
"Wait until there is space (or we're shutting down, in which case give up)":
```cpp
not_full_.wait(lk, [this] {
    return queue_.size() < capacity_ || closed_;
});
```

### The lesson
The predicate answers: **"What condition, once true, means I can stop waiting?"**
Not: "What condition makes me wait?"

Two valid reasons to stop waiting, joined by `||`:
- The *normal* progress condition (space available / item available)
- The *give up* condition (closed)

Then after waking, check which one it was.

---

## Mistake 3: Empty lambda capture

### What I wrote
```cpp
[] { return queue_.size() < capacity_ || closed_; }
```

### Why it's wrong
`[]` captures nothing. Inside a member function, `queue_`, `capacity_`, `closed_` are all `this->queue_`, etc. The lambda doesn't know about `this` unless you tell it.

### The fix
```cpp
[this] { return queue_.size() < capacity_ || closed_; }
```

`[this]` captures the `this` pointer by value, so member access works. `[&]` works too but is less explicit.

### The lesson
A lambda is a separate object. It only sees what you explicitly hand it via the capture list.

---

## Mistake 4: Pushing after waking from closure

### What I had (after fixing the predicate)
```cpp
not_full_.wait(lk, [this] { return queue_.size() < capacity_ || closed_; });
queue_.push_back(item);
not_empty_.notify_one();
return true;
```

### The bug
The predicate has **two reasons to be true**:
1. Space opened up (good, push the item)
2. Buffer was closed (bad, don't push, return false)

My code barrelled ahead and pushed either way. It would insert into a full, closed buffer and return `true` claiming success.

### The fix
```cpp
not_full_.wait(lk, [this] { return queue_.size() < capacity_ || closed_; });
if (closed_) return false;   // check WHY we woke up
queue_.push_back(item);
not_empty_.notify_one();
return true;
```

### The lesson
After `wait` returns, the predicate is true — but **you need to check which disjunct is true**. The predicate tells you "you may proceed"; the post-wait check tells you "proceed with what?"

---

## Mistake 5: `return false` from a function returning `optional<T>`

### What I wrote in `pop`
```cpp
if (closed_) return false;
```

### Why it's wrong
`pop`'s return type is `std::optional<T>`. `false` is a `bool`. This won't compile cleanly (or will compile to something nonsensical depending on `T`).

### The fix
```cpp
return std::nullopt;
```

### The lesson
`std::nullopt` is the "empty" value for `std::optional`. It's the idiom for "no result to return."

---

## Mistake 6: `pop` gave up on close even when items remained

### What I wrote
```cpp
not_empty_.wait(lk, [this] { return queue_.size() > 0 || closed_; });
if (closed_) return std::nullopt;   // ← too aggressive
```

### The bug
Scenario:
1. Producer pushes 3 items.
2. Producer calls `close()`. Now `closed_ == true`, queue has 3 items.
3. Consumer calls `pop()`. Predicate true (both conditions hold). Wakes up.
4. `if (closed_)` is true → consumer returns `nullopt` without taking any of the 3 items.

I just threw away 3 items that were legitimately produced.

### The fix
The spec (which I had already written in the comment!) says:
> "Returns nullopt if buffer is closed **AND** empty."

```cpp
if (closed_ && queue_.empty()) return std::nullopt;
```

### The subtle asymmetry between push and pop
- `push`: closed → immediately give up (no point pushing into a closed buffer)
- `pop`: closed → drain remaining items first, only give up when truly empty

This makes sense: closing means "no more will be *produced*." It does *not* mean "throw away what's already there."

---

## The big concept: what `notify` actually means

The deepest lesson from this session, worth writing down in full.

A `notify` **is not a message.** It doesn't mean "go ahead." It doesn't mean "a resource is available for you." It means:

> **"The world may have changed. Re-check your assumptions."**

That's why the predicate loop exists. Three separate things can happen between "I was notified" and "I actually hold the lock and am running":

### 1. Spurious wakeup
The OS can wake a waiting thread even when nobody called `notify`. Rare, but the standard permits it. If I'd used `if` instead of `while`, a spurious wakeup would make me charge ahead on an unchanged state.

### 2. Stolen wakeup (the big one)
Scenario: buffer empty, 2 consumers waiting. Producer pushes 1 item, calls `notify_one()`. Consumer C1 is marked runnable. Before C1 actually acquires the lock, a brand-new consumer C3 walks in, calls `pop()` directly (not via wait), grabs the lock first, takes the item. C1 finally gets the lock — queue is empty again. Without the predicate loop, C1 would call `front()` on an empty queue → crash.

This isn't a spurious wakeup. C1 was legitimately woken. But by the time C1 ran, the world had changed. **Anyone calling `pop()` fresh can "steal" a wakeup intended for a waiter**, because `notify_one` doesn't atomically give the lock to the notified thread.

### 3. Broadcast excess
`close()` uses `notify_all`. If 5 consumers are waiting and only 2 items remain, all 5 wake up — 3 will find an empty queue. Without the predicate loop, those 3 would crash.

### One mechanism, three bugs prevented
`while (predicate_false) { wait; }` (or the `wait(lk, pred)` overload that does this internally) handles all three.

### The unifying rule
> **The condvar is just an attention mechanism. The shared state (behind the lock) is the source of truth.**

You cannot use a condvar to "signal an event." You can only use it to say "the predicate *might* be true now — go look."

---

## `notify_one` vs `notify_all`

General rule:

- **`notify_one`**: "One slot's worth of change happened — wake one waiter." Use this after a single push or single pop.
- **`notify_all`**: "A global state change happened that all waiters need to know about." Use this on `close()`.

If I'd used `notify_one` in `close()`, only one waiter per condvar would wake up. The rest would sleep forever → program hangs on exit.

---

## Fairness is NOT guaranteed in C++

When multiple threads wait on the same condvar and one `notify_one` fires, **which one wakes up?**

The C++ standard says: **unspecified.** Might be FIFO, might be LIFO, might be "whatever the OS felt like."

In practice on Linux+glibc it's roughly FIFO (futex queue), but you must not rely on this. Your code must be correct under any wakeup order.

This is why:
- The lecture calls out "mutexes and semaphores are not fair in C++."
- **FIFO semaphores** are a separate concept that has to be built explicitly.
- Starvation in dining philosophers is a real risk even with a correct (deadlock-free) algorithm.

---

## The final working shape

```cpp
template <typename T>
class BoundedBuffer {
public:
    explicit BoundedBuffer(std::size_t capacity) : capacity_(capacity) {}

    bool push(T item) {
        std::unique_lock<std::mutex> lk{mut_};
        not_full_.wait(lk, [this] {
            return queue_.size() < capacity_ || closed_;
        });
        if (closed_) return false;
        queue_.push_back(std::move(item));
        not_empty_.notify_one();
        return true;
    }

    std::optional<T> pop() {
        std::unique_lock<std::mutex> lk{mut_};
        not_empty_.wait(lk, [this] {
            return !queue_.empty() || closed_;
        });
        if (closed_ && queue_.empty()) return std::nullopt;
        T item = std::move(queue_.front());
        queue_.pop_front();
        not_full_.notify_one();
        return item;
    }

    void close() {
        std::scoped_lock lock(mut_);
        closed_ = true;
        not_full_.notify_all();
        not_empty_.notify_all();
    }

private:
    std::size_t capacity_;
    std::deque<T> queue_;
    std::mutex mut_;
    std::condition_variable not_full_;
    std::condition_variable not_empty_;
    bool closed_ = false;
};
```

---

# Part 2: The semaphore variant

Same `BoundedBuffer<T>` interface, different primitives: `std::counting_semaphore<>` for `spaces_sem_` (empty-slot tickets) and `items_sem_` (item tickets), plus a plain `std::mutex` to protect the queue's internal structure. No condition variables. The lecture pseudocode translates almost 1:1.

## Semaphore-journey at a glance

| Stage | Mistake | What it taught me |
|---|---|---|
| 7 | Plain `bool closed_` read without the mutex | C++ memory model: mixed-mode access is UB even on `bool`. TSan catches it; casual testing doesn't. |
| 8 | `close()` only set `closed_`, released nothing | Semaphores have no `notify_all`. Parked threads don't wake on a flag flip — they wake on a permit. |
| 9 | `items_sem_.acquire()` where I meant `.release()` | `acquire` is WAIT, `release` is SIGNAL. They're symmetric in a diagram, opposite in effect. |
| 10 | Single `N_enough` constant shared by both semaphores | The two sides have independent worst-case waiter counts. Sizing them with `max()` over-releases harmlessly; sizing them too small deadlocks. |

Each expanded below.

---

## Mistake 7: data race on `closed_`

### What I had
```cpp
// close()
{ std::scoped_lock lock(mut_); closed_ = true; }

// push() — reads WITHOUT holding mut_
spaces_sem_.acquire();
if (closed_) { ... }
```

### Why it's wrong
One writer under a mutex, one reader without. Per the C++ memory model, a non-atomic read that races with a write is **undefined behavior** — even on a type that happens to be 1 byte. You can get:
- A torn read (compiler splits the access).
- A cached read that never sees the update.
- The compiler reordering across the check and deciding `if (closed_)` is loop-invariant.

### The fix
```cpp
std::atomic<bool> closed_ = false;
```
Now every access is atomic with `memory_order_seq_cst` by default. All call sites (read in push/pop, write in close) continue to work unchanged.

### The alternative fix I rejected
Reading `closed_` inside the mutex everywhere would also work, but it would force `push`'s "re-check after acquire" to lock the mutex just for a bool check — ugly and high-contention. Atomic `bool` is the cleaner tool for "a flag read from many, written by one."

### The lesson
**Mixed-mode access is always UB, regardless of type size.** If a field is read outside a lock anywhere, it must be atomic (or you must read it inside the lock everywhere).

---

## Mistake 8: `close()` only set the flag

### What I had
```cpp
void close() {
    { std::scoped_lock lock(mut_); closed_ = true; }
    // done — that's it
}
```

### Why it's wrong
In the condvar version, `notify_all()` wakes every parked thread so each can re-check its predicate. Semaphores have **no broadcast primitive**. A thread parked on `spaces_sem_.acquire()` is not going to wake up because a flag somewhere flipped — it wakes up if and only if someone releases a permit.

Consequence: any producer blocked on a full buffer, or any consumer blocked on an empty buffer, at the moment `close()` runs → parked forever → thread never returns → `main()`'s `join()` hangs → process hangs.

### The fix
Release *enough* permits on each semaphore so every possibly-parked waiter wakes up, re-checks `closed_`, and bails.

```cpp
spaces_sem_.release(MAX_PRODUCERS_WAITERS);
items_sem_.release(MAX_CONSUMERS_WAITERS);
```

### Why "enough" is a finite, knowable number
Worst case: every producer is parked on `spaces_sem_` and every consumer on `items_sem_`. That upper bound is `NUM_PRODUCERS` and `NUM_CONSUMERS` respectively. Release that many and every waiter is guaranteed to wake.

### Why over-releasing is safe (the self-healing trick)
If `close()` releases more permits than there are parked waiters, the extras don't corrupt anything. They sit in the semaphore. When a later `pop()` (or `push()`) consumes one, the code path sees `closed_ == true`, releases the permit *back*, and returns nullopt / false. The permit is reusable by the next would-be waiter, who does the same dance. Eventually everyone bails and the extras sit unused forever — harmless.

**This is why the "release on bail" line in push/pop matters.** Without it, over-released permits get silently consumed and the system can't heal.

### The lesson
> A condvar is an attention mechanism. A semaphore is a permit counter. Shutdown with a condvar is one line: broadcast. Shutdown with a semaphore requires you to know, at shutdown time, the maximum number of possibly-parked waiters per direction.

---

## Mistake 9: `acquire()` where I meant `release()`

### What I had
```cpp
for (int i = 0; i < N_enough; ++i) {
    spaces_sem_.release();
    items_sem_.acquire();   // <-- intended release()
}
```

### The behaviour this produces
`acquire()` is the wait. In a close() running when the buffer is empty:
- `items_sem_.acquire()` blocks waiting for a permit that will never arrive.
- `close()` itself hangs inside the for-loop.
- `main()` never gets back to `join()`.
- Whole program hangs.

If the buffer has items, `close()` consumes their permits without ever actually draining the items. Consumers starve, `Consumed < Produced`.

### Why the mistake was tempting
`release` and `acquire` are perfectly symmetric in any diagram of a semaphore. Producers acquire, consumers release. Producers release (the other semaphore), consumers acquire (the other semaphore). It's easy to type the wrong one.

### The pattern that would have caught it
Read the code out loud in plain English:
- "acquire" → "I need a permit, block until I get one"
- "release" → "I'm giving out a permit, wake someone"

In `close()`, the intent is **"wake everyone up"** = give out permits = release. "Wait for something" = acquire has no role here.

### The lesson
For semaphores specifically: trace the direction of permit flow. Producers *consume* space permits and *emit* item permits. Consumers *consume* item permits and *emit* space permits. `close()` *emits* permits on both sides (to wake parked consumers of those permits). If you're consuming when the intent is emitting, you've typed the wrong one.

---

## Mistake 10: a single `N_enough` shared between both semaphores

### What I had (first pass)
```cpp
constexpr int N_enough = 2;   // one number for both sides
for (int i = 0; i < N_enough; ++i) {
    spaces_sem_.release();
    items_sem_.release();
}
```
With `NUM_PRODUCERS = 3` and `NUM_CONSUMERS = 2`, `spaces_sem_` got only 2 releases — one producer could be left parked. Deadlock on shutdown whenever the buffer was full at close time.

### Intermediate fix (better but not minimal)
```cpp
int N_enough = std::max(MAX_CONSUMERS_WAITERS, MAX_PRODUCERS_WAITERS);
for (int i = 0; i < N_enough; ++i) { ... }
```
Correct — guarantees enough permits on both sides — but over-releases on whichever side has fewer waiters. Harmless in practice (the self-healing trick absorbs the extras) but it's symptomatic of not separating the two concerns.

### The right shape
```cpp
constexpr int MAX_PRODUCERS_WAITERS = NUM_PRODUCERS;
constexpr int MAX_CONSUMERS_WAITERS = NUM_CONSUMERS;
// ...
spaces_sem_.release(MAX_PRODUCERS_WAITERS);
items_sem_.release(MAX_CONSUMERS_WAITERS);
```
Two independent numbers, one `release(n)` call each. Uses the `counting_semaphore::release(n)` overload instead of a loop.

### The design smell
Using `std::max` to collapse two independent quantities into one is almost always wrong. When you feel the temptation, ask: *"are these two quantities actually the same thing?"* If the answer is "no, they just happen to both need an upper bound" — keep them separate.

### A real-code improvement (not done yet)
`MAX_*_WAITERS` are `main()`-level facts, not buffer-level. Ideally the buffer takes them as constructor parameters rather than reaching up to constants it shouldn't know about. For this learning exercise the shortcut is fine, but the architectural smell is worth naming.

### The lesson
Semaphore sizing is **per-direction**. Producers wait on one side, consumers on the other, and the two counts are independent. Collapse them with care.

---

## The big concept: semaphores don't have broadcast

The unifying insight from Part 2:

> A condvar's `notify_all()` is a free built-in "wake every waiter on this condition." A semaphore has nothing comparable. The only way to wake N waiters is to emit N permits. Which means **the shutdown path must know the maximum number of parkable waiters per direction at shutdown time**.

This is a real design burden condvars don't impose — and it's the single biggest reason the lecture warns you against assuming semaphores are "the simpler primitive."

The compensating pattern is **self-healing release on bail**: whenever a waiter wakes, observes closed, and decides to give up, it re-releases the permit. That way:
- Over-releasing permits → harmless, they get recycled and eventually sit unused.
- Under-estimating the count → deadlock.
- Exact-sizing → guaranteed wake, no waste.

You basically always prefer "over-size slightly + rely on self-healing" to "count waiters precisely at runtime," because the former is robust to mistakes and the latter is fragile.

---

## The big trade-off: condvar vs semaphore

| Aspect | Condvar + predicate loop | Two semaphores |
|---|---|---|
| Normal-path LOC | Longer (predicate, while-loop, re-check) | Shorter (acquire / do work / release) |
| Spurious wakeups | Must handle via predicate loop | Irrelevant — `acquire` doesn't return spuriously |
| Shutdown LOC | 1 line: `notify_all` on each CV | Must release N permits per side + self-healing logic in push/pop |
| Shutdown fragility | Can't get it "wrong" in a subtle way | Easy to under-release → hang, or skip the re-release → leak |
| State visibility for debugging | Predicates make "what it's waiting for" explicit | Permit count is invisible — you can't print it |
| Idioms match | Lecture `wait(cv)`/`signal(cv)` pseudocode | Lecture `wait(sem)`/`signal(sem)` pseudocode directly |
| Mental model | "Wake up and re-check the world" | "Permit-based resource accounting" |

**Short version:**
- **Condvars**: verbose steady state, cheap shutdown. Hard to get subtly wrong; easy to get *verbosely* wrong.
- **Semaphores**: cheap steady state, verbose shutdown. Easy to get subtly wrong at the edges.

### When each actually wins
- Condvars win when the "wait condition" is a complex predicate over multiple state variables — you can express it directly.
- Semaphores win when you're naturally thinking in **resource counts** (connection pools, rate limits, bounded queues with no other state). Encoding "there are K things available" as a semaphore count is beautiful.
- For producer-consumer specifically, *either* works well. The condvar version reads more like the lecture invariants ("while full, wait"); the semaphore version reads more like the hardware analogy ("take a ticket"). Worth writing both at least once — it's the cleanest way to *feel* the trade-off.

---

## Final working shape (semaphore version)

```cpp
constexpr int MAX_PRODUCERS_WAITERS = NUM_PRODUCERS;
constexpr int MAX_CONSUMERS_WAITERS = NUM_CONSUMERS;

template <typename T>
class BoundedBuffer {
public:
    explicit BoundedBuffer(std::size_t capacity)
        : capacity_(capacity),
          spaces_sem_(static_cast<std::ptrdiff_t>(capacity)),
          items_sem_(0) {}

    bool push(T item) {
        spaces_sem_.acquire();
        if (closed_) { spaces_sem_.release(); return false; }
        {
            std::unique_lock<std::mutex> lk{mut_};
            queue_.push_back(std::move(item));
        }
        items_sem_.release();
        return true;
    }

    std::optional<T> pop() {
        items_sem_.acquire();
        std::unique_lock<std::mutex> lk{mut_};
        if (closed_ && queue_.empty()) {
            items_sem_.release();
            return std::nullopt;
        }
        T item = std::move(queue_.front());
        queue_.pop_front();
        spaces_sem_.release();
        return item;
    }

    void close() {
        { std::scoped_lock lk(mut_); closed_ = true; }
        // no broadcast available — release enough permits per side
        spaces_sem_.release(MAX_PRODUCERS_WAITERS);
        items_sem_.release(MAX_CONSUMERS_WAITERS);
    }

private:
    std::size_t capacity_;
    std::deque<T> queue_;
    std::mutex mut_;
    std::counting_semaphore<> spaces_sem_;
    std::counting_semaphore<> items_sem_;
    std::atomic<bool> closed_ = false;
};
```

Notes:
- `closed_` is `std::atomic<bool>` — mandatory given it's read outside the mutex.
- In `pop()`, `spaces_sem_.release()` is inside the `unique_lock` scope for simplicity. Correct, but you could lift it out for slightly less lock contention.
- `close()` uses `release(n)` — the counted overload that replaces the old for-loop.

---

## Semaphore-specific checklist (in addition to the condvar one below)

- [ ] Is every state field that the hot path reads outside the mutex **atomic**? (Plain `bool` is a trap.)
- [ ] Does `close()` release enough permits on **both** semaphores for every possibly-parked waiter?
- [ ] Do I know the **worst-case waiter count per direction**, or am I collapsing them with `max()`?
- [ ] Does every "closed" bail-out (in push and in pop) **re-release** the permit it consumed?
- [ ] Did I reach for `acquire()` when I meant `release()` (or vice versa)? Read the function out loud.
- [ ] Am I holding the mutex any longer than strictly needed? Semaphore operations should generally be outside the lock where safe.

---

## Sabotage experiments to run on the semaphore version

1. Set `MAX_PRODUCERS_WAITERS = 1` and bump `NUM_PRODUCERS` to 5 → `close()` eventually hangs when the buffer is full.
2. Revert `closed_` to a plain `bool` and run under TSan → immediate race report.
3. Remove the `spaces_sem_.release()` in `push`'s closed-bail path → extra permits leak on shutdown; eventually a later arriving producer consumes one, tries to push into a closed buffer, and `return false`s without cleaning up.
4. In `close()`, swap `release` for `acquire` on either semaphore → program hangs inside `close()` itself on an empty buffer.
5. Change the `pop` check from `closed_ && queue_.empty()` back to `closed_` alone → every still-queued item after close gets discarded; `Consumed < Produced`.

Each takes 30 seconds, each teaches a distinct invariant.

---

## What Part 2 cemented on top of Part 1

1. **Primitive choice shapes *where* the complexity lives.** Condvars push complexity into the steady-state predicate. Semaphores push it into shutdown.
2. **Both versions are the same algorithm.** If you can't state the algorithm in English without naming primitives, you haven't understood it yet.
3. **TSan catches the race classes that normal testing misses** — specifically mixed-mode access on "small" types like `bool`. Use it before declaring anything "done."
4. **Self-healing release-on-bail is not optional in the semaphore version.** It's the thing that makes over-sizing `close()` releases safe. Missing it converts "over-released" into "leaked."

---

## Checklist I'll use on the next sync problem

Before writing any condvar code:

- [ ] Can I state the predicate in plain English using the word **"until"**?
- [ ] Does my predicate have a *progress* disjunct AND a *give up* disjunct?
- [ ] After `wait` returns, do I distinguish which disjunct was satisfied?
- [ ] Is my lambda capturing `this` (or using `[&]`)?
- [ ] Am I signaling the **other** condvar (the one a waiter might be blocked on) after making progress?
- [ ] Does shutdown use `notify_all` on every relevant condvar?
- [ ] Is my return type consistent on all paths?
- [ ] If I change `while` to `if`, can I construct a scenario that breaks it? (If no, maybe I don't need the while — but I probably do.)

---

## Sabotage experiments to run next time (cement the lessons)

1. Change `while` (predicate) to `if` — run under stress. Crashes eventually on empty-queue pop.
2. Change `notify_all` in `close()` to `notify_one` — run with multiple waiters. Program hangs.
3. Remove the `if (closed_) return false;` in `push` — run. Items may be pushed after shutdown with `true` return, violating the contract.

Each takes 30 seconds to try and gives you a visceral feel for *why* the correct version is written the way it is.

---

## What I'd do differently next time

1. **Write the English predicate first.** Don't touch code until I can say "wait until X" out loud.
2. **Trace one complete scenario on paper** before compiling: "producer pushes at time t1, consumer pops at t2, close at t3 — what happens at each line?"
3. **Predict before running.** Guess what `Produced=` and `Consumed=` will print. If my prediction is wrong, find out why *before* debugging.
4. **Check my own comments against my code.** I wrote the right spec in the comment for `pop` ("closed AND empty") — then didn't match it in the code. The comment was correct; my code was the bug.
