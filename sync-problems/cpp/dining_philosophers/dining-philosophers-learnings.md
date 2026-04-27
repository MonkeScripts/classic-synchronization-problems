# Dining Philosophers (C++): Mistakes & Learnings

A record of implementing dining philosophers in C++20 across three strategies — `std::scoped_lock`, footman semaphore, and Tanenbaum state-tracking — the bugs along the way, and the conceptual points each one forced me to internalise.

---

## The headline observation

Three families of takeaways, in ascending order of transferability:

1. **Sharing matters more than I expected.** Most of my early bugs weren't about algorithm — they were about *where the synchronization primitive lives*. A `std::mutex` declared inside `eat()` is a private mutex per call. A semaphore declared inside `eat()` is a private semaphore per call. Both are useless. The synchronization happens only when the primitive is *the same object* across threads.

2. **Static analysis vs runtime invariants — the footman + TSan lesson.** ThreadSanitizer's `lock-order-inversion` analysis is purely structural — it builds a graph of "who locks what while holding what" and reports any cycle. The footman strategy passes correctness because the *runtime semaphore* breaks the cycle by participation (pigeonhole), not by lock ordering. TSan can't see that prevention argument. It flagged the cycle. That's a true property of the lock-acquire graph, and it's a beautiful instance of "the analyzer is right about what it can see; what it can't see is the whole prevention argument."

3. **Tanenbaum: state-tracking instead of resource-locking.** A whole different paradigm. No chopstick mutexes. Per-philosopher CV (or semaphore) gated by per-philosopher state. The signal that wakes a hungry philosopher comes from a *neighbor* finishing, not from acquiring something. This is a different mental model from everything in producer-consumer / readers-writers / barrier — it's the first problem in this set where the solution is "no resource is locked, only the right to claim it is announced."

---

## The journey at a glance

| # | Mistake | Fix | Category |
|---|---|---|---|
| 1 | `eat_scoped_lock` used a single global `std::mutex mut` (one lock, one philosopher at a time — no chopsticks at all) | Add `std::array<std::mutex, N> chopsticks;` and lock `chopsticks[left], chopsticks[right]` | algorithm |
| 2 | Declared `chopsticks` and `num_eaters` *inside* `eat_*` functions (stack-local) — every philosopher got private copies, no mutual exclusion | Promote both to namespace scope | C++ scoping |
| 3 | `chopsticks[left].lock(); chopsticks[right].lock();` with NO matching `unlock()` (and the surrounding `{}` block doesn't help — `std::mutex` isn't RAII) | Add manual `unlock()` calls in reverse acquire-order, or wrap in `std::lock_guard` | C++ RAII |
| 4 | Initial unlock order was `left, right` (same as lock) | Conventional: reverse-of-acquire — `right, left` | convention |
| 5 | Layered `std::scoped_lock` *inside* the footman variant — defended against deadlock twice | Sequential `lock()`, `lock()` — trust the footman cap | conceptual |
| 6 | Tanenbaum `safe_to_eat`: `algo_states[pid] == State::EATING;` — comparison, not assignment | One character: `=` not `==` | typo |

---

## Deep dive: mistake 1 — scoped_lock with one global mutex

### What I wrote first

```cpp
std::mutex mut;   // ONE global

void eat_scoped_lock(...) {
    std::scoped_lock<std::mutex> lk{mut};   // serialize EVERYTHING
    ...
}
```

This compiles, runs, and passes the invariant check. **It's also wrong** — not because it's incorrect (no philosopher eats while a neighbor eats) but because it has no concurrency. One philosopher eats at a time. The dining-philosophers problem has been replaced by "five threads taking turns at a shared resource."

### The mistake I was making

I'd internalized "use a mutex" without internalizing what the mutex is *guarding*. In dining philosophers the resources are the **chopsticks**, not the eating action. A mutex per chopstick lets non-adjacent philosophers eat simultaneously (e.g., 0 and 2 share no chopstick). A single global mutex collapses that down to one.

### What `scoped_lock`'s superpower actually is

`std::scoped_lock` is *not* a fancier `lock_guard`. Its variadic form takes **multiple mutexes** and acquires them atomically with `std::lock`'s internal try-and-back-off algorithm — that's the whole point.

```cpp
std::scoped_lock lk{chopsticks[left], chopsticks[right]};
```

This is what makes strategy 3 different from strategy 1 (naive). Without `scoped_lock`, the natural transcription is:
```cpp
chopsticks[left].lock();
chopsticks[right].lock();
```
which is the textbook deadlock — five threads grab left, all wait for right, full cycle. `scoped_lock` quietly avoids that by re-trying the acquire if it can't get all locks at once.

### The transferable lesson

> **Identify the resource being protected before reaching for the primitive.** If your "mutex" guards everything, you've serialized everything. Match one mutex to one logical resource.

---

## Deep dive: mistake 2 — locals where globals were needed

### What I wrote

```cpp
void eat_footman(std::size_t pid, std::mt19937& rng) {
    std::array<std::mutex, N> chopsticks;            // LOCAL
    std::counting_semaphore<> num_eaters{N-1};       // LOCAL
    ...
    num_eaters.acquire();
    chopsticks[left].lock();
    chopsticks[right].lock();
    ...
}
```

### Why it does nothing

Each call to `eat_footman` allocates a fresh array of mutexes on the stack and a fresh semaphore. **No two philosophers ever touch the same mutex.** Philosopher 0's `chopsticks[0]` and philosopher 1's `chopsticks[0]` are different objects in different stack frames. The semaphore: each philosopher gets their own private "at most N−1 of me can eat" — meaningless because there's only ever 1 of them.

The lock acquires succeed instantly (no contention). `num_eaters.acquire()` always succeeds. Adjacent philosophers can eat simultaneously and `check_invariant` fires — the only reason it might not is timing luck.

### Why this fooled me twice

The fix moved `chopsticks` to global scope, but I left `num_eaters` local — same bug, second iteration. The asymmetry between two declarations on consecutive lines was easy to miss. Lesson: when fixing a "this should be global" bug, look at *every other thing in the same scope* to see if it has the same problem.

### The transferable lesson

> **A synchronization primitive only does work when it's a shared object across threads.** A mutex/semaphore declared in function scope is per-call. Always ask: "is this primitive visible to every thread that needs to coordinate?"

---

## Deep dive: mistake 3 — `lock()` without `unlock()`

### What I wrote

```cpp
{
    chopsticks[left].lock();
    chopsticks[right].lock();
    ... eat ...
}   // closing brace does NOTHING for these locks
```

The braces *look* like they should release at scope exit, but `std::mutex::lock()` returns nothing — there's no RAII helper bound to the scope. You need:
- `std::lock_guard<std::mutex> lg{mu};` — RAII auto-release at scope end, or
- `mu.lock(); ... mu.unlock();` — manual.

### What happens at runtime

Philosopher 0 locks `chopsticks[0]` for meal 1, never unlocks. Either:
- On meal 2, philosopher 0 tries to lock `chopsticks[0]` again → `std::mutex` is non-recursive → **UB**, in practice deadlock, or
- Philosopher 4 needs `chopsticks[0]` (its right) and blocks on meal 1 already.

Either way, hang.

### The fix

```cpp
chopsticks[left].lock();
chopsticks[right].lock();
... eat ...
chopsticks[right].unlock();
chopsticks[left].unlock();   // reverse-of-acquire
```

I went with manual unlock to keep the textbook-footman shape visible. `lock_guard` would be more idiomatic C++ but hides the lock-left-then-right structure that's pedagogically the *point* of the footman strategy.

### The transferable lesson

> **`std::mutex` has no RAII semantics by itself.** Plain braces don't unlock; only `lock_guard` / `unique_lock` / `scoped_lock` do. If you wrote `mu.lock()`, you owe `mu.unlock()`.

---

## Deep dive: mistake 5 — `scoped_lock` inside footman

### What I had

```cpp
void eat_footman(...) {
    num_eaters.acquire();
    std::scoped_lock lk{chopsticks[left], chopsticks[right]};   // try-and-back-off
    ...
}
```

This is correct but **conceptually muddled**. The textbook footman is:

```
acquire footman ticket           ← cap N-1 contenders
lock chopsticks[left]            ← one at a time
lock chopsticks[right]           ← one at a time
... eat ...
release in reverse
release footman ticket
```

The cap of N−1 *itself* is the deadlock-prevention argument — pigeonhole: with at most N−1 hungry philosophers and N chopsticks, at least one philosopher always has both neighbors free, so no full cycle can form. **Sequential lock-left-then-right is safe** because the semaphore alone breaks the cycle.

Adding `scoped_lock` layers a second deadlock-avoidance mechanism on top. Correct, but hides the textbook lesson — you can't tell anymore whether the cap or the back-off algorithm is the thing keeping you out of deadlock.

### The transferable lesson

> **When learning a strategy, write it in its purest form.** The compound version "footman + scoped_lock" works, but the pedagogical point of the footman is the pigeonhole argument; layering scoped_lock obscures it. After you understand each strategy in isolation, *then* compose.

---

## Deep dive: mistake 6 — Tanenbaum `==` for `=`

### What I wrote

```cpp
void tanenbaum_safe_to_eat(std::size_t pid) {
    if (algo_states[pid] == State::HUNGRY &&
        algo_states[left] != State::EATING &&
        algo_states[right] != State::EATING) {
            algo_states[pid] == State::EATING;   // <-- comparison, discarded
            tanenbaum_cv[pid].notify_one();
        }
}
```

Line 4 inside the if-body uses `==` instead of `=`. The comparison evaluates to a bool, the bool is discarded — `algo_states[pid]` never changes.

### Effect at runtime

- Philosopher 0 enters `take_forks`, sets `algo_states[0] = HUNGRY`, calls `tanenbaum_safe_to_eat(0)`.
- The if-predicate evaluates true (no neighbors EATING yet).
- Inside: bug runs, state stays HUNGRY. Notify fires on `tanenbaum_cv[0]`, but no one's waiting yet.
- The cv.wait predicate `algo_states[0] == State::EATING` returns false → philosopher 0 sleeps.
- All five philosophers do this. **Total deadlock on first meal.** `min=0` everywhere.

### The compiler would have caught this

`-Wall -Wextra` produces `-Wunused-comparison` ("expression result unused") for a comparison-as-statement. The CMakeLists' default flags didn't fire this warning loudly enough that I noticed. **Lesson: turn warnings up.** A typo this character-cheap shouldn't reach the runtime.

---

## Big lesson: TSan lock-order-inversion as a structural false positive

After fixing every bug above, I ran `dining_philosophers` under `-DSANITIZER=thread`. All three strategies completed (50 meals each, no invariant violations). But TSan reported **one warning, in `eat_footman`** — `lock-order-inversion (potential deadlock)`. The cycle:

```
philosopher 0: lock chopstick 0, then lock chopstick 1
philosopher 1: lock chopstick 1, then lock chopstick 2
philosopher 2: lock chopstick 2, then lock chopstick 3
philosopher 3: lock chopstick 3, then lock chopstick 4
philosopher 4: lock chopstick 4, then lock chopstick 0   ← closes the cycle
```

This is **the canonical naive-strategy deadlock cycle**, and it's a true cycle in the lock-acquire graph TSan builds. TSan correctly identified it. The footman semaphore prevents the cycle from *actualizing* — pigeonhole, N−1 contenders cannot form a full cycle of N — but **TSan's analysis is purely structural**. It sees the cycle, doesn't see the runtime cap.

The other two strategies don't trigger this:
- `eat_scoped_lock`: `std::lock`/`std::scoped_lock` uses try-lock-and-back-off, which TSan recognizes as deadlock-safe — no edges added to its graph.
- `eat_tanenbaum`: no chopstick mutexes at all, so no graph.

### The transferable lesson

> **TSan can detect *structural* deadlock potential. It cannot reason about *quantitative* arguments like "at most N−1 → no full cycle possible."** False positive in this case, but a useful one: it tells you "your code's lock-acquire graph has a cycle; you'd better have an external argument for why that cycle can't close." For the footman, that argument is the semaphore. For the asymmetric-order variant, it'd be "philosopher 4 reverses the order" — which TSan *would* recognize because the lock edges go the other way.

This connects directly to the readers-writers fairness-gap finding from the recap: the algorithm is correct only up to the assumptions of its analyzer. TSan + footman is a clean, friendly version of the same lesson.

---

## Big lesson: Tanenbaum is state-tracking, not resource-locking

The Tanenbaum strategy is qualitatively different from the others.

| | scoped_lock / footman | Tanenbaum |
|---|---|---|
| What's protected | chopsticks (resources) | the right to enter EATING |
| Whose action wakes you | scheduler / lock release | a *neighbor* finishing their meal |
| Mutex protects | chopstick state | per-philosopher state array |
| CV / semaphore role | none / cap | gate per philosopher |

In Tanenbaum:
- No chopstick mutexes at all.
- One global mutex protects the per-philosopher state array `algo_states[]`.
- Each philosopher has their own CV (or semaphore — see below) that's signaled when their right to eat is granted.

The big shift: instead of *acquiring resources*, you *announce intent*, then *wait for permission*. The permission comes from a neighbor finishing, calling `safe_to_eat` on their neighbors as a courtesy.

### The CV-based version (what I implemented)

```cpp
void tanenbaum_safe_to_eat(std::size_t pid) {
    // PRECONDITION: caller holds tanenbaum_mutex.
    if (algo_states[pid]   == State::HUNGRY
     && algo_states[left]  != State::EATING
     && algo_states[right] != State::EATING) {
        algo_states[pid] = State::EATING;
        tanenbaum_cv[pid].notify_one();
    }
}

void eat_tanenbaum(std::size_t pid, std::mt19937& rng) {
    // take_forks
    {
        std::unique_lock lock(tanenbaum_mutex);
        algo_states[pid] = State::HUNGRY;
        tanenbaum_safe_to_eat(pid);
        tanenbaum_cv[pid].wait(lock, [pid]{
            return algo_states[pid] == State::EATING;
        });
    }
    // eat (mutex released — no neighbor can enter EATING because
    // their test() will see algo_states[pid] == EATING and refuse)
    states[pid].store(State::EATING);
    check_invariant(pid);
    meal_counts[pid].fetch_add(1);
    jitter(rng, 3);
    states[pid].store(State::THINKING);
    // put_forks
    {
        std::unique_lock lock(tanenbaum_mutex);
        algo_states[pid] = State::THINKING;
        tanenbaum_safe_to_eat(left);
        tanenbaum_safe_to_eat(right);
    }
}
```

Critical properties:
- The **lambda overload** of `cv.wait` handles the predicate-loop. If `safe_to_eat(pid)` succeeds inside `take_forks`, `algo_states[pid]` is already EATING when we reach `wait` — the predicate returns true and `wait` doesn't block. If we *did* need to wait, the predicate-loop catches spurious wakeups. Same lesson as producer-consumer.
- The mutex is **released during eating**, but no neighbor can transition to EATING because their test() sees `algo_states[pid] == EATING` and refuses. The state, not the lock, holds your place.
- `notify_one` ≡ `notify_all` here because each `tanenbaum_cv[i]` has at most one waiter.
- Tanenbaum is **deadlock-free but NOT starvation-free**. Two philosophers on either side of a hungry one can alternate forever. The fix is a PRIORITY/WAITING state that biases the test against repeating eaters; not implemented here.

---

## The semaphore-based Tanenbaum (alternative formulation, walkthrough)

The textbook (Tanenbaum's *Modern Operating Systems*) uses **per-philosopher semaphores** instead of CVs. It's worth understanding because the semaphore version exposes the underlying mechanism — "signal as a deposit in a counter" — more cleanly than the CV version. The shape is:

```
takeChopsticks(i):
    wait(mutex)
    state[i] = HUNGRY
    safeToEat(i)              // maybe signals s[i], maybe doesn't
    signal(mutex)
    wait(s[i])                // <-- behavior depends on what safeToEat did

putChopsticks(i):
    wait(mutex)
    state[i] = THINKING
    safeToEat(left)
    safeToEat(right)
    signal(mutex)

safeToEat(i):
    if state[i] == HUNGRY
       && state[left]  != EATING
       && state[right] != EATING:
       state[i] = EATING
       signal(s[i])
```

`s[i]` is **per-philosopher, initialized to 0**, and it's a *signaling* semaphore, not a *protection* one.

### Walkthrough — philosopher 2 with neighbors 1 and 3

1. P2 calls `takeChopsticks(2)`, grabs the mutex, sets `state[2] = HUNGRY`.
2. P2 calls `safeToEat(2)`. Suppose `state[1] == EATING`. Condition fails. No signal. State stays HUNGRY.
3. P2 releases the mutex, calls `wait(s[2])` — blocks (counter is 0).
4. Time passes. P1 finishes eating, calls `putChopsticks(1)`.
5. P1 sets `state[1] = THINKING`, calls `safeToEat(2)` (its right neighbor).
6. Now in `safeToEat(2)`: `state[2] == HUNGRY` ✓, `state[1] != EATING` ✓ (just set to THINKING), `state[3] != EATING` ✓. Condition holds. Set `state[2] = EATING`, `signal(s[2])`.
7. P2's `wait(s[2])` unblocks. P2 eats.

So the "retry mechanism" isn't a retry by P2 itself — it's neighbors *actively waking P2 up* when they put down their chopsticks. P2 only needs to be woken once, and only when conditions are actually right.

### Why this design is clever

The two cases at the end of `takeChopsticks` are **exhaustive**:

- **Case A: neighbors weren't eating.** `safeToEat(i)` ran the body, set `state[i] = EATING`, called `signal(s[i])`. Counter on `s[i]` goes from 0 to 1. Then `wait(s[i])` decrements 1 → 0 and **does not block** — philosopher proceeds straight to eating.
- **Case B: a neighbor was eating.** `safeToEat(i)` did nothing. Counter on `s[i]` stays at 0. `wait(s[i])` blocks. Later, the neighbor finishes and calls `safeToEat(i)` from inside their `putChopsticks`, which signals — and now this philosopher unblocks.

There's no third case where I block forever, *assuming neighbors eventually finish eating* (a "no philosopher eats forever" liveness assumption baked into the problem).

### The semantics of `s[i]`

`s[i]` is **not** initialized to 1. It's initialized to 0. Each `s[i]` is a per-philosopher *signaling* semaphore — its job is "wake me when I can eat," not "protect a resource." That's why `wait(s[i])` blocks on the first call: the counter starts at 0.

Compare to `bibo` in the readers-writers code, which started at 1 because its job was mutual exclusion. Different jobs, different initial values. **The semaphore's initial value is part of expressing what it means.**

### The key property: semaphores have memory

The `wait(s[i])` happens *outside* the mutex (after `signal(mutex)`). That's deliberate — you don't want to block while holding the global mutex, or no one could ever update state to wake you up. But it creates a small window: between `signal(mutex)` and `wait(s[i])`, what if a neighbor finishes eating and signals me?

Answer: **it's fine, because semaphores have memory.** `signal(s[i])` increments the counter to 1. When I then call `wait(s[i])`, I find counter == 1, decrement to 0, and proceed without blocking. The signal isn't "lost."

This is a key property of semaphores that makes this kind of pattern work — and it's why the textbook chose semaphores rather than condition variables. **Condition variables lose signals if no one's waiting; semaphores don't.**

### Same-semaphore insight (the load-bearing observation)

`s[i]` is **the same object** whether philosopher *i* is calling `wait(s[i])` on themselves, or a neighbor is calling `signal(s[i])` from inside their `putChopsticks`. They're both manipulating the same counter.

Mailbox metaphor:
- Philosopher *i* is the only one who ever reads the mailbox (`wait(s[i])`).
- Neighbors (philosophers *i−1* and *i+1*) are the ones who drop notes in (`signal(s[i])`), via their `putChopsticks` calling `safeToEat(i)`.
- Philosopher *i* might also drop a note in their own mailbox — if `safeToEat(i)` succeeds during their own `takeChopsticks`, meaning they could eat immediately.

The semaphore counter is the count of unread notes. `wait` says "give me a note, blocking until one arrives." `signal` says "drop a note in." Multiple actors can drop notes; one actor reads them.

If `s[i]` were somehow per-philosopher-private, neighbors couldn't wake you. **The fact that the waiter and the signaler reference the same semaphore is what lets information flow between threads.** This is what semaphores (and mutexes, condvars, channels) ARE: shared objects whose state is a communication channel between threads.

### Why one straight-line sequence absorbs both cases

Notice what's NOT in `takeChopsticks`:

- No `if` checking whether to wait or not.
- No retry loop.
- No "did `safeToEat` succeed?" return value.

The philosopher writes one straight-line sequence: mark hungry, try to take, release mutex, wait for green light. **The semaphore's counter absorbs the difference between "green light already given" and "green light not yet given."** From the philosopher's point of view, both feel the same — they just wait and eventually proceed.

This is the pattern: **encode the condition in the semaphore's counter rather than in branching control flow.** It works because semaphores have memory (a signal that arrives early is remembered, not lost), so the waiter doesn't need to know whether the signal already happened or is yet to come.

I've now seen this pattern twice in this curriculum:
1. **Readers-writers**: `roomEmpty`'s counter encoded "is the room currently claimable" — writers and the first/last reader manipulated it, and other threads acquired it without needing to know who would release.
2. **Tanenbaum here**: `s[i]`'s counter encodes "does philosopher i have permission to eat" — neighbors and the philosopher themselves manipulate it, and the philosopher waits on it without needing to know who'll signal.

The semaphore is a shared piece of state that lets one thread say "the condition holds" and another thread say "wake me when it does," **without either needing to coordinate timing**. That's the whole game.

---

## Final results

Run with N=5, MEALS_PER_PHILOSOPHER=50, jitter ≤ 3ms.

| Strategy | Time | Meals (per philosopher) | Spread | TSan |
|---|---|---|---|---|
| `eat_scoped_lock` | 244 ms | 50 50 50 50 50 | 0 | clean |
| `eat_footman` | 326 ms | 50 50 50 50 50 | 0 | **1 warning** (lock-order-inversion, see below) |
| `eat_tanenbaum` | 257 ms | 50 50 50 50 50 | 0 | clean |

- **30-run regular-build stress: 30 ok / 0 fail.** No deadlocks, no invariant violations.
- **TSan run completes the program.** All three strategies finish cleanly with the same 50/50/50/50/50 result. The `eat_footman` warning is a structural false positive — see the deep-dive above.

Spread of 0 doesn't mean "perfect fairness" here — it means each philosopher hit the hard cap of 50 meals before another philosopher fell behind. To actually expose Tanenbaum starvation numerically, flip the run to be time-bound (eat as much as you can in X ms) and bias jitter so two philosophers eat fast around a slow middle one.

---

## Cross-references

- `sync-problems/go/dining_philosophers/dining-philosophers-learnings.md` — the Go companion. Two strategies: odd/even ring (asymmetric ordering, lecture slide 31) and try-and-back-off (channel equivalent of `std::scoped_lock`). Includes the cross-language analyzer comparison (TSan flags the C++ footman; `-race` is happy with both Go strategies).
- `sync-problems/cpp/readers_writers/readers-writers-learnings.md` — for the separation-of-algorithm-state-from-invariant-oracle principle, which Tanenbaum reuses (`algo_states[]` vs `states[]`).
- `sync-problems/cpp/producer_consumer/producer-consumer-learnings.md` — for the predicate-loop / "what `notify` actually means" mental model the Tanenbaum CV version reuses.
- `sync-problems/docs/recap.md` — for the cross-cutting "primitive choice shapes where complexity lives" and "correctness vs liveness" themes that connect this writeup to the others.

---

## Next moves

- [ ] **Time-bound the run** to expose Tanenbaum starvation numerically. Replace the `MEALS_PER_PHILOSOPHER` loop cap with a `steady_clock::now() < deadline` predicate; run for 500 ms; compare per-philosopher meal counts.
- [ ] **Add the asymmetric strategy** (`eat_asymmetric`): philosopher N−1 grabs right-first, breaking the lock-acquire graph cycle. Predict: TSan is happy with this version (no cycle), and the program runs without deadlock without needing a footman.
- [ ] **Implement the naive `eat()` stub** (line 79 in main.cpp) and watch it deadlock — confirms the textbook claim and makes concrete what each of the other strategies is buying you.
- [ ] **Try the semaphore-based Tanenbaum** (`std::counting_semaphore<1>` per philosopher instead of `std::condition_variable`). Measure whether the C++ counting-semaphore fairness gap from readers-writers shows up here too. Probably yes; this is the same primitive that flaked under turnstile load.
- [ ] **Sabotage experiments to cement the lessons:**
  - Move `chopsticks` back inside `eat_footman`. Predict: invariant fires. Confirm.
  - Remove `num_eaters.acquire()` from `eat_footman`. Predict: program deadlocks (this is the strategy-1 naive shape). Confirm.
  - Replace `cv.wait(lock, predicate)` with `cv.wait(lock)` (no predicate) in Tanenbaum. Predict: works most of the time, but a spurious wakeup eventually fires `check_invariant` because someone resumes too early. Confirm under stress.
  - Change Tanenbaum's `safe_to_eat(left); safe_to_eat(right);` to test only one neighbor. Predict: one philosopher eventually starves when only one side ever gets tested.

Each is a 30-second edit, predict-then-observe — fastest way to internalize each invariant.
