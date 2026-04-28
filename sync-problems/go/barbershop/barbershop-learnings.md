# Barbershop (Go): Mistakes & Learnings

A record of porting the barbershop problem from C++ to Go, the bugs along the way, and a few language-fundamentals points the port forced me to internalize. Companion to `cpp/barbershop/barbershop-learnings.md`.

---

## The headline observation

The C++ version taught me **the protocol** — two handshakes (front + back), each a signal-then-wait pair on each side. The Go port taught me **how to express that protocol with channels**, plus a handful of Go-language gotchas that have nothing to do with concurrency:

1. **Channels replace semaphores naturally — but the rendezvous structure is the same.** A `chan struct{}` is a one-shot synchronization primitive: send blocks until receive, receive blocks until send. Each unbuffered channel = one rendezvous slot. The 4-semaphore protocol becomes a 4-channel protocol with the same `release ↔ acquire` pairing renamed to `send ↔ receive`.

2. **`select` + `default:` is the non-blocking-channel-op idiom.** A `select` with a `default:` clause never blocks: if no case is ready *right now*, `default` fires immediately. This is exactly the balk semantics — "walk in, no seat available *immediately*, leave."

3. **A back handshake can be slide 44's order or its inverse — both are valid.** As long as both sides flip symmetrically, the four-event rendezvous still pairs up. Two valid shapes, same correctness.

4. **`for {}` is Go's `loop {}` — almost.** Both are infinite loops you exit from inside. Go's `for {}` is a *statement* (no value). Rust's `loop {}` is an *expression* — `break value` makes the loop yield a value. That's why the dining-philosophers Rust solution could lift mutex guards out of a retry loop with `let (l, r) = loop { … break (l, r); };` and Go can't.

---

## The journey at a glance

| # | Mistake | Fix | Category |
|---|---|---|---|
| 1 | `case default:` | `default:` (no `case` keyword) | Go grammar |
| 2 | `for { select { ... } }` where every case `return`s | Drop the `for` — `select { ... }` alone | dead loop |
| 3 | Barber `return` at end of cut case → exits after one customer | Drop the `return` — let `for` carry control back | shutdown vs continue |
| 4 | Barber back-handshake order matched slide 44 (`signal barberDone; wait customerDone`); customer order had also been changed → both sides at sends, mutual deadlock | Either restore slide 44 on both sides, OR keep the inverted order on both sides — must be symmetric | rendezvous |
| 5 | `case id := <-bs.chairs:` with `id` never used | `case <-bs.chairs:` (discard) — Go errors on unused locals | Go strictness |
| 6 | First instinct: "shouldn't `for { select }` make my customer non-blocking?" | The `for` doesn't help; `default:` inside `select` is the actual non-blocking knob | Go idiom |
| 7 | Buffered channel of size `Chairs` thinking it modeled "total in shop" (matches C++) | The buffer only counts customers *waiting to be picked up*; once the barber receives, they're out of the buffer. So total-in-shop = buffer + (1 if barber busy) | semantics |

---

## Mistake 1: `case default:` is a syntax error

### What I wrote
```go
select {
case <-bs.shutdown:
    return false
case bs.chairs <- id:
    // ...
case default:                  // ❌
    return false  // balk
}
```

### What's wrong
`default` is a Go *keyword*, not an identifier. The `select`/`switch` grammar has two arm forms: `case <expr>:` and `default:`. They are separate productions — `default` doesn't go through `case`.

### The fix
```go
default:
    return false  // balk
```

### Why this matters
The first instinct as a C++/Java programmer is to write `case default:` because in those languages `default` is a `case`-arm-like clause. Go's grammar is closer to a state machine: every `select`/`switch` has at most one `default:` arm, syntactically distinct from any `case`. Once that lands, the rule is mechanical: `case <something>:` for the conditional arms, bare `default:` for the catch-all.

---

## Mistake 2: redundant `for { select { ... } }`

### What I wrote
```go
func (bs *Barbershop) Customer(id int) bool {
    for {
        select {
        case <-bs.shutdown:
            return false
        case bs.chairs <- id:
            // ... full success path
            return true
        default:
            return false
        }
    }
}
```

### What's wrong
Every arm of the `select` ends with `return`. The `for` loop body always exits via `return` on the first iteration. The loop never iterates a second time.

### The fix
Drop the `for`:
```go
func (bs *Barbershop) Customer(id int) bool {
    select {
    case <-bs.shutdown:
        return false
    case bs.chairs <- id:
        // ... success path
        return true
    default:
        return false
    }
}
```

### When `for { select { ... } }` *is* the right shape
The barber's `Barber()` function genuinely needs it:
```go
func (bs *Barbershop) Barber() {
    for {
        select {
        case <-bs.shutdown:
            return                 // exit only on this arm
        case <-bs.chairs:
            // serve one customer ...
            // (no return — fall through to the next loop iteration)
        }
    }
}
```

The barber serves *many* customers over its lifetime, exiting only on shutdown. The `for { select }` shape is right when:
- The goroutine handles multiple events over time, AND
- At least one arm doesn't return (so the loop has work to do on iteration N+1).

When every arm returns, the loop is dead code.

---

## Mistake 3: barber `return` after one customer

### What I wrote
```go
case <-bs.chairs:
    bs.barberReady <- struct{}{}
    time.Sleep(2 * time.Millisecond)
    bs.barberDone <- struct{}{}
    <-bs.customerDone
    return                       // ❌ exits the whole function
```

### What's wrong
`return` inside a `case` arm exits the **whole function**, not just the `select` arm. After serving customer 0, the barber goroutine is gone. Customer 1's send to `bs.chairs` succeeds (buffer has capacity 3) but `<-bs.barberReady` blocks forever — nobody's left to signal them.

### The fix
Just delete the `return`. After the case body finishes, control implicitly falls through to the bottom of the `for` body, which loops back and re-enters `select`.

### Why I wrote it
Probably habit from the C++ version where `barber()` was already inside a `while (!stop_)` loop and each `return` would have looked similar. In Go, `case <whatever>:` is *not* a function-level construct — there's no `break` needed at the end (Go's `switch`/`select` cases don't fall through by default), but there's also no implicit `return`. Just write the work and let the loop do its thing.

### The general pattern: only `return` from arms that *should* exit
Every arm of a long-running worker `select` is one of:
- **Exit arm** (e.g., `case <-shutdown:`) — `return` here.
- **Work arm** (e.g., `case <-bs.chairs:`) — do the work, then implicit fall-through to the next iteration.

Mismatching this is the bug. In a `for { select }` shape, work arms should never `return`.

---

## Mistake 4: back-handshake direction inversion (deadlock-via-symmetry-mismatch)

### What I had at one point
```go
// Customer:
bs.customerDone <- struct{}{}    // signal "I'm done"
<-bs.barberDone                  // wait for "you can leave"

// Barber:
bs.barberDone <- struct{}{}      // signal "you can leave"
<-bs.customerDone                // wait for "I'm done"
```

### What's wrong
On unbuffered channels, both sides start with a *send*. Each send blocks until a corresponding receive is ready. Neither goroutine has reached its receive yet — both are at the send. **Deadlock.**

### The fix that actually shipped
I inverted the *customer* side too, which restored symmetry the other way:
```go
// Customer:
<-bs.barberDone                  // wait for "you can leave"
bs.customerDone <- struct{}{}    // signal "I'm done"

// Barber:
bs.barberDone <- struct{}{}      // signal "you can leave"
<-bs.customerDone                // wait for "I'm done"
```

Now the rendezvous pairs:
1. Barber sends `barberDone` ↔ customer receives `barberDone`.
2. Customer sends `customerDone` ↔ barber receives `customerDone`.

### The deeper observation: two valid orderings, both correct

**Slide 44's order** (the "I'm done first, then you say go"):
```
Customer: signal customerDone   →   Barber: wait customerDone
Barber:   signal barberDone     →   Customer: wait barberDone
```

**My inverted order** (the "barber says go first, then I confirm done"):
```
Barber:   signal barberDone     →   Customer: wait barberDone
Customer: signal customerDone   →   Barber: wait customerDone
```

Both are valid four-event rendezvous. The semantic shifts slightly ("done-then-ack" vs "ack-then-done"), but as a synchronization primitive, both pair up cleanly.

**The structural rule the bug violated:** *both sides must agree on the order.* If one side does `signal-then-wait` and the other side does `signal-then-wait` on the *opposite* channel, both block on their first sends. The two sides need to be **mirror images of each other**, not the *same shape*. Customer-signal pairs with barber-wait; customer-wait pairs with barber-signal.

A clearer way to think about it: every channel in the protocol has *exactly one sender and one receiver*. If you correctly assign roles per channel, the order on each side falls out of the channel direction. The bug came from changing my mind on direction halfway through and updating only one side.

---

## Mistake 5: unused variable `id`

### What I wrote
```go
case id := <-bs.chairs:
    bs.barberReady <- struct{}{}
    // ... id never read
```

### What the compiler said
```
./main.go:85:8: declared and not used: id
```

### What's wrong
Go treats unused locals as *compile errors*, not warnings. There's no equivalent of `[[maybe_unused]]` or Rust's `_` prefix to silence it — unused locals simply don't compile.

### Three valid fixes
```go
case <-bs.chairs:                  // discard the value entirely
case id := <-bs.chairs:            // and then use id later (e.g., log it)
case _ = <-bs.chairs:              // explicit discard with the blank identifier
```

The bare `<-bs.chairs:` form is the cleanest when the value isn't needed. `_` is for "I'm telling the compiler I deliberately don't want this." The `id` form is only valid if `id` actually appears in the case body.

### Why Go is strict about this
Idiomatic Go treats unused variables as a code smell — "if you bound it, you meant to use it; otherwise the binding is noise." This catches a real category of bug at compile time: forgetting to update a use-site after refactoring. The strictness has a price (you can't temporarily comment out a use during exploration without also commenting out the binding), but the runtime category of "stale assignment lying around" goes away.

(Same philosophy as gofmt: Go would rather force one canonical way than allow stylistic debate. The compiler errors are the enforcement mechanism for what other languages would handle with linters.)

---

## Mistake 6: thinking `for { ... }` would make `Customer` non-blocking

### The misconception
"I want the customer to balk if no chair is free. Surely if I wrap the work in `for { select { ... } }`, the loop won't block, right?"

### Why this is wrong
The `for` only controls *iteration*. The blocking still happens inside `select`. Without a `default:` arm, `select` blocks until at least one case is ready — exactly the same way it would without the `for`. Wrapping a blocking construct in a loop doesn't change its blocking behavior; it just makes you do the blocking thing repeatedly.

### The actual non-blocking knob: `default:` inside `select`
```go
select {
case bs.chairs <- id:
    // queued
default:
    // would have blocked — balk immediately
}
```

`default:` makes the entire `select` non-blocking. If no case is ready *right at the moment of evaluation*, `default` fires.

### The pattern recognition
> If I want non-blocking channel ops, I need `select { case ... default: ... }`. The `for` is irrelevant to blocking; it only controls retry/iteration shape.

### A subtle gotcha worth knowing
If multiple cases in a `select` are ready simultaneously, Go picks **pseudo-randomly** among them — *not* in source order. So:

```go
select {
case <-bs.shutdown:                  // a
    return false
case bs.chairs <- id:                // b
    // serve
default:                             // c
    return false
}
```

If `bs.shutdown` is closed *and* `bs.chairs` has space, the customer might enter the chair (case b) instead of seeing the shutdown (case a). Probably fine for most harnesses (the customer just got served right before shutdown took effect), but if you need ordering, you have to write it explicitly:

```go
// Check shutdown first, then attempt chair seat.
select { case <-bs.shutdown: return false; default: }
select { case bs.chairs <- id: /* serve */; default: return false }
```

(This is rarely needed in practice, but worth knowing the semantic.)

---

## Mistake 7: buffered-channel capacity ≠ "total customers in shop"

### What I assumed (from the C++ version)
The C++ shop tracked `customers_` as "everyone in the shop, including the one being cut." Max value: `CHAIRS = 3`. The balk check was `if (customers_ == CHAIRS) return false;`.

### What Go actually does
```go
chairs: make(chan int, Chairs)   // buffered chan, capacity 3
```

A buffered channel holds values that have been *sent but not yet received*. The barber's `<-bs.chairs` *removes* the customer from the buffer. So the buffer count only tracks customers **waiting to be picked up**, not "total in shop."

### The semantic difference
| State | Buffer count | Total in shop |
|---|---|---|
| Empty shop | 0 | 0 |
| 1 cutting, 0 waiting | 0 | 1 |
| 1 cutting, 3 waiting | 3 | 4 |
| 1 cutting, 4 waiting | 3 (4th balks) | 4 |

The Go shop with `make(chan int, 3)` allows **3 waiting + 1 being cut = 4 total** before any balking. The C++ shop with `customers_ == 3` allows max 3 total. Different model.

### How to match the C++ semantics in Go
Use `make(chan int, Chairs - 1)` if you want strict parity (waiting-only slots). Or add an explicit counter with a mutex:
```go
type Barbershop struct {
    mu       sync.Mutex
    inShop   int   // total, like C++ customers_
    // ...
}
```
The mutex+counter version mirrors the C++ shape exactly. The buffered-channel version is more idiomatic Go but with subtly different semantics.

### The bigger lesson
> Channel capacity and "logical occupancy" aren't the same thing. The buffered channel models "queued, not yet handed off." If your spec talks about "total occupants," you need an explicit counter — the channel won't track it for you.

---

## Side-by-side: Go vs C++ vs Rust on this problem

| Concern | C++ | Go | Rust (slide 44 shape) |
|---|---|---|---|
| Synchronization primitive | `std::counting_semaphore<>` | `chan struct{}` | `tokio::sync::Semaphore` or `Notify` |
| Pairing rule | one releaser, one acquirer | one sender, one receiver per channel | one releaser, one acquirer (or signaler / awaiter) |
| Non-blocking try | `try_acquire()` | `select { default: }` | `try_lock()` / `try_acquire()` |
| Mutex around counter | `std::mutex + std::unique_lock` | `sync.Mutex + Lock/Unlock` (or send to a channel) | `tokio::sync::Mutex` |
| Shutdown signal | `std::atomic<bool> + sem.release()` | `close(chan struct{})` (idiomatic) | `tokio::sync::Notify` or AtomicBool |

The **structure** of the protocol — two handshakes, four signal-pairs, one mutex around the counter, one shutdown signal — is identical across all three languages. The *primitives* and the *idioms for non-blocking variants* differ.

---

## Summary: the Go-specific muscle memory

After this problem, the patterns I want to internalize:

1. **`default:` (no `case` keyword) is the non-blocking knob in `select`.** `for` controls iteration, not blocking.
2. **`return` from a `select` arm exits the function**, not just the arm. Long-running workers with a `for { select }` shape should only `return` from exit arms (e.g., `<-shutdown`); work arms fall through implicitly.
3. **Both sides of a rendezvous must mirror each other**, not have the same shape. One side does `send→recv`, the other does `recv→send`. If both start with a send, you've deadlocked.
4. **Unused locals are compile errors**, not warnings. Use `_` or the bare `<-ch` form to discard.
5. **Buffered channel capacity ≠ logical occupancy.** The buffer counts "queued, not yet received." If your spec needs "total," add a counter.
6. **`for {}` and `loop {}` are siblings, not twins.** Both are infinite loops; only Rust's `loop` is an expression that can yield a value through `break`. Go relies on flag variables or labeled break for similar idioms.
7. **`gofmt -w main.go`** is the canonical formatter — there's exactly one canonical Go style, and the tool enforces it. No formatting debates.
