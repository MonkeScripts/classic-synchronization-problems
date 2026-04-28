# Barbershop (C++): Mistakes & Learnings

A record of implementing the sleeping-barber problem in C++20 with the slide-44 four-semaphore protocol, the bugs along the way, and the one conceptual realization that made the whole thing click.

---

## The headline observation

The barbershop solution is **two rendezvous bookending a haircut**, plus a mutex-protected counter. Once you see it as two handshakes, everything else is mechanical.

Most of my bugs traced back to having only the *back* handshake (`customer_done_sem_` ↔ `barber_done_sem_`) wired up, and missing the *front* handshake (`customer_sem_` ↔ `barber_sem_`). The customer was queuing, sleeping "through the haircut," and signaling done — without ever waiting for the barber to wave them in. The protocol passed the served+balked counter test even though the *physical model* was broken: with CHAIRS=3, three "haircuts" could happen simultaneously while the barber was still parked on `customer_sem_`.

The fix was a one-character pairing flip on each side, but the conceptual fix was naming the missing thing: **the front handshake**.

---

## The two handshakes, side by side

```
Front handshake (start of haircut):
  Customer: signal(customer_sem_)   →  "I'm here"
  Barber:   wait(customer_sem_)     →  "got it"
  Barber:   signal(barber_sem_)     →  "your turn"
  Customer: wait(barber_sem_)       →  "thanks, sitting down"

Back handshake (end of haircut):
  Customer: signal(customer_done_sem_)   →  "I'm done sitting"
  Barber:   wait(customer_done_sem_)     →  "got it"
  Barber:   signal(barber_done_sem_)     →  "you can leave"
  Customer: wait(barber_done_sem_)       →  "thanks, leaving"
```

Each handshake is a **signal-then-wait pair on each side**, with the two semaphores going in opposite directions. The customer signals one, waits on the other; the barber waits on the first, signals the second.

Once you see this pattern, the whole solution is just two rendezvous bookending the haircut, plus a mutex for the counter. That's the entire structure.

(If neither party does anything during the haircut, you can collapse the back handshake to a single semaphore. But the front handshake is essential because the customer genuinely needs to wait until the barber is ready before sitting in the chair.)

### The structural invariant the protocol enforces

Every semaphore has **exactly one releaser and exactly one acquirer**:

| Semaphore | Customer | Barber | Means |
|---|---|---|---|
| `customer_sem_` | `release` | `acquire` | "a customer arrived" |
| `barber_sem_` | `acquire` | `release` | "barber is ready" |
| `customer_done_sem_` | `release` | `acquire` | "customer finished" |
| `barber_done_sem_` | `acquire` | `release` | "barber acked" |

If your code violates this — both sides releasing, both sides acquiring, or one side touching neither — *that semaphore isn't synchronizing anything*. It's either accumulating permits forever or stuck at zero. Both are bugs.

---

## The journey at a glance

| # | Mistake | Fix | Category |
|---|---|---|---|
| 1 | Customer skipped `barber_sem_.acquire()` between signaling arrival and "haircut" | Add the front-handshake wait — customer waits until barber waves them in | protocol |
| 2 | Barber wrote `barber_sem_.acquire()` instead of `release()` (signaler vs. waiter inverted) | Flip to `release()` — barber is the signaler on this semaphore | direction |
| 3 | "What if I init `barber_sem_{1}` and treat it as a resource lock?" | Doesn't fix anything — barber acquires + releases its own permit, no rendezvous with anyone | conceptual |
| 4 | Customer wrote `barber_sem_.release()` instead of `acquire()` (added line, wrong direction) | Flip to `acquire()` — same one-releaser/one-acquirer rule | direction |
| 5 | `std::this_thread::sleep_for(1ms)` — the placeholder needed naming | Don't use POSIX `sleep(1)` (1 *second*, requires `<unistd.h>`) | C++ stdlib |
| 6 | Barber loop didn't recheck `stop_` after `customer_sem_.acquire()` — hung at shutdown | Insert `if (stop_.load()) break;` between the wake and the cut | shutdown |
| 7 | Default config (CHAIRS=3, 20ms inter-arrival, 1ms cut) never balks → balk path untested | Increase contention to actually exercise the full-shop case | testing |

---

## Mistake 1: missing front-handshake wait on the customer side

### What I wrote first
```cpp
bool customer(int customer_id) {
    {
        std::unique_lock<std::mutex> lk{mut_};
        if (customers_ == CHAIRS) return false;
        customers_ += 1;
    }
    customer_sem_.release();           // "I'm here"
    std::this_thread::sleep_for(1ms);  // "haircut" — but I never waited for the barber
    customer_done_sem_.release();
    barber_done_sem_.acquire();
    // ...
}
```

### What's wrong
The customer signaled their arrival (`customer_sem_`) and then started the "haircut" sleep immediately, with no rendezvous in between. There was no point in the protocol where the customer was *blocked waiting for the barber*. Multiple customers (up to CHAIRS) could be sleeping their haircut simultaneously while the barber was still parked on `customer_sem_.acquire()`.

### The fix
Insert `barber_sem_.acquire()` between the arrival signal and the haircut:

```cpp
customer_sem_.release();      // "I'm here"
barber_sem_.acquire();        // "wait for barber to wave me in"
std::this_thread::sleep_for(1ms);
```

### Why it passed the unit test anyway
The harness only checks `served + balked == TOTAL_CUSTOMERS`. Every non-balking customer eventually wakes from `barber_done_sem_` because the back handshake is correctly wired and the barber loops once per `customer_sem_` permit. The counts balance. But the *protocol* is wrong: simultaneous haircuts are physically impossible with one barber.

### The diagnostic to prove it
Add `std::atomic<int> active_haircuts{0};` and increment/decrement around the customer's `sleep_for`. Print if it ever exceeds 1.

- Buggy version: prints values up to `CHAIRS` (3 in this configuration).
- Fixed version: stays pinned at 1.

The unit test wasn't sensitive enough to catch this — only an instrumented invariant was.

---

## Mistake 2: barber side had `acquire` where it should have been `release`

### What I wrote first
```cpp
void barber() {
    while (!stop_.load()) {
        customer_sem_.acquire();   // "a customer arrived"
        barber_sem_.acquire();     // ❌ supposed to signal, not wait
        std::this_thread::sleep_for(1ms);
        customer_done_sem_.acquire();
        barber_done_sem_.release();
    }
}
```

### What's wrong
The barber is the **signaler** on `barber_sem_` ("I'm ready for the next one"), not the waiter. With `acquire()` here and the original init of 0, the barber blocks forever on the first iteration — nobody is signaling.

### The fix
```cpp
customer_sem_.acquire();
barber_sem_.release();        // "I'm ready for the next one — sit"
std::this_thread::sleep_for(1ms);
```

### The pairing rule that catches this category
Once I had the table from the headline section in my head, this category of bug becomes mechanical:

> For every semaphore, name **who signals** and **who waits**. Then `release()` only goes on the signaler's side and `acquire()` only goes on the waiter's side.

If your code has both sides releasing or both sides acquiring on the same semaphore, you've broken the structural invariant.

---

## Mistake 3: the "barber_sem_{1} as a resource lock" detour

### What I tried
```cpp
std::counting_semaphore<> barber_sem_{1};   // init 1 — "barber is available"

void barber() {
    while (!stop_.load()) {
        customer_sem_.acquire();
        barber_sem_.acquire();   // claim the "barber" resource
        std::this_thread::sleep_for(1ms);
        customer_done_sem_.acquire();
        barber_done_sem_.release();
        barber_sem_.release();   // release the "barber" resource
    }
}
```

The mental model: `barber_sem_` is a resource pool of size 1 representing barber availability, with the barber claiming and releasing it around its own work.

### Why it doesn't work
Trace what `barber_sem_` does in this version:

- Barber acquires it. Count: 1 → 0.
- Barber does cut work (no other thread observes the semaphore).
- Barber releases it. Count: 0 → 1.

Net: count starts and ends at 1, observed by no one else. **It's a no-op self-lock.** You could delete both lines and the program would behave identically.

### The deeper conceptual issue
There are two coherent ways to model `barber_sem_`:

1. **Signal model** (slide 44): init 0, barber releases ("ready"), customer acquires ("my turn"). Each haircut = one matched release/acquire pair.
2. **Resource model**: init 1, customer acquires ("I claim the barber"), someone (the barber, after the cut) releases. The customer is the one waiting on availability.

What I'd done was *neither*: a resource model where the resource was claimed and released by the same thread, without anyone else interacting with the semaphore.

### Why I dropped the resource model entirely
Two reasons the signal model is better here:

1. **Symmetry with `customer_sem_`.** Treating both front-handshake semaphores as signals gives a uniform mental model: every semaphore is one side telling the other "now."
2. **The four-semaphore protocol becomes provably a 1:1 rendezvous.** Each customer's flow synchronizes with exactly one barber iteration by construction.

The resource model is implementable but you'd have to redesign the back handshake, and the lecture didn't go that way for a reason.

---

## Mistake 4: missing front-handshake wait on the customer side, take two

### What I wrote
After learning that the customer needs to participate in `barber_sem_`, my next attempt was:

```cpp
customer_sem_.release();
barber_sem_.release();           // ❌ released, didn't wait
std::this_thread::sleep_for(1ms);
```

### Why it's wrong
Same direction-inversion category as mistake 2, mirrored. With the barber also releasing, **both sides release, neither acquires**. The `barber_sem_` count just grows monotonically and never blocks anyone. Same observable behavior as mistake 1: simultaneous haircuts.

### The fix and the lesson
`acquire()`, not `release()`. After this round, the rule of thumb stuck:

> If I'm adding a missing semaphore call, the very first thing I should ask is *who signals and who waits on this semaphore?* Get the role right before the line is even written. Otherwise I'll add a syntactically correct line that breaks the structural invariant.

---

## Mistake 5: `sleep(1)` would have been a 1-second delay

### What I had
```cpp
sleep(1); // getting hair cut()
```

### What's wrong
There is no free function `sleep` in standard C++. `sleep(unsigned int seconds)` is POSIX, declared in `<unistd.h>`. On Linux + libstdc++ it's often visible via transitive includes from `<thread>`, so this might compile silently — but it sleeps for **1 second**, not 1ms. With 30 customers that's 30+ seconds of wall time, and the timing-sensitive arrival pattern (mean 20ms) would degenerate into "customers arrive, shop fills up, balks for the next 30 seconds."

### The fix
```cpp
std::this_thread::sleep_for(1ms);
```

The file already imports `<chrono>` and brings in `using namespace std::chrono_literals;` at line 28, so `1ms` works as a duration literal. This is the same shape the dining-philosophers C++ files use.

### The bigger lesson
When standard C++ has a portable, scoped, well-typed answer (`std::this_thread::sleep_for` with a `std::chrono::duration`), don't reach for the POSIX freestanding function — it's untyped (`unsigned int`), implicitly seconds, and only works on POSIX platforms. The C++ version also nests cleanly with the `chrono_literals` ergonomics: `1ms`, `500us`, `2s` all just work.

---

## Mistake 6: shutdown hung because the loop didn't recheck `stop_`

### What I had
```cpp
void stop() { stop_.store(true); customer_sem_.release(); }   // wakes the barber

void barber() {
    while (!stop_.load()) {
        customer_sem_.acquire();
        barber_sem_.release();
        std::this_thread::sleep_for(1ms);
        customer_done_sem_.acquire();   // ❌ blocks forever after shutdown
        barber_done_sem_.release();
    }
}
```

### What's wrong
After all customers join in `main()`, `shop.stop()` runs:
1. `stop_ = true`.
2. `customer_sem_.release()` — gives the sleeping barber a fake permit so it can wake up.

The barber wakes, but **the loop's `stop_` check happens at the top, *before* `customer_sem_.acquire()`.** The check already passed for this iteration. The barber proceeds straight into the cut sequence:

- Releases `barber_sem_` (no waiter).
- Sleeps 1ms.
- `customer_done_sem_.acquire()` — blocks forever, because no real customer is going to release this.

Result: `barber_thread.join()` in `main()` never returns. Program hangs.

### The fix
Insert the recheck *between* the acquire and the work:

```cpp
void barber() {
    while (true) {
        customer_sem_.acquire();
        if (stop_.load()) break;   // wake-from-shutdown exit
        barber_sem_.release();
        std::this_thread::sleep_for(1ms);
        customer_done_sem_.acquire();
        barber_done_sem_.release();
    }
}
```

### The general pattern
Any "long-running worker on a semaphore queue + shutdown signal" needs **a recheck between the wake and the work**. The wake-up is a coarse signal — it could mean "real work" or "time to exit" — and the loop has to disambiguate before committing to a cut sequence that depends on real customers being present.

This is the same shape as the producer-consumer poison-pill pattern: producer pushes a sentinel that the consumer recognizes after waking, and the consumer breaks rather than processing the sentinel as data. Different surface (sentinel value vs. boolean flag), same idea.

---

## Mistake 7: the balk path was never exercised

### What I observed
The first clean run printed:
```
Served=30 Balked=0 Total=30 (expected 30)
```

The counter check passed. But notice: **zero balks.** The harness's `if (customers_ == CHAIRS) return false;` line was never executed. With CHAIRS=3, 1ms cuts, and mean 20ms inter-arrival, the barber is so much faster than arrivals that the queue never fills.

### Why this matters
Three invariants live in the file's header comment:
- (A) `customers_` always matches actual number of customers inside shop.
- (B) no customer "lost."
- (C) `served + balked == TOTAL_CUSTOMERS`.

Invariant (A) and the balk-path mutex critical section (lines 39–44) are *only exercised* when the shop is full. Under the default config they're dead code — the program could pass with `customers_ == CHAIRS` swapped for `customers_ == 999` and nothing would notice.

### The fix (for testing, not for code)
Crank up contention until balks happen. Three knobs:
- Faster arrivals: `arrival(1.0 / 2.0)` for mean 2ms gaps.
- Slower cuts: `sleep_for(10ms)` to make the barber a bottleneck.
- More customers: `TOTAL_CUSTOMERS = 200+` to ride bursts.

Run with `BUILD-TSAN/barbershop` to confirm that the balk-path mutex critical section is also race-free. Passing once means little; passing 10,000 iterations under TSan with mixed served/balked outcomes is real evidence.

### The bigger lesson
**A passing test exercising one half of the protocol isn't a passing test of the protocol.** Dual paths (success vs. balk, hit vs. miss, present vs. absent) need *both arms* exercised before you can say "the protocol works." Default-config runs are coverage holes in disguise — they look green but only because the configuration didn't push hard enough to surface the other half.

---

## Summary: the C++-specific muscle memory

After this problem, the patterns I want to internalize:

1. **Two handshakes, not one.** Bookend the haircut: front (start) and back (end). Each handshake is two semaphores going in opposite directions, with each side doing one signal and one wait.
2. **Per-semaphore, name who signals and who waits.** Then `release` only goes on the signaler's side and `acquire` only goes on the waiter's side. If both sides do the same op, the semaphore isn't synchronizing anything.
3. **Init-value follows the role.** Signal-style semaphores init to 0 (nobody has signaled yet). Resource-style semaphores init to capacity. Picking the wrong init is usually a sign you've also picked the wrong role assignment.
4. **`std::this_thread::sleep_for(1ms)`, not `sleep(1)`.** The C++ stdlib answer is portable, typed, and lives next to `chrono_literals`.
5. **Shutdown signal needs a recheck *after* the wake.** Worker loops wake up on the same primitive used for real work; the recheck disambiguates "real work" from "exit now."
6. **A passing test of one path is not a passing test.** If your protocol has a balk arm, exercise the balk arm. If you can't see balks in the output, the test is too easy.
