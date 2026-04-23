# Producer-Consumer in Go (channels): Mistakes & Learnings

Companion to the C++ learnings doc. Same problem, different primitive, different mistakes. This is the most compressed of the three variants — the "bounded buffer" itself is a single `make(chan string, N)` call — but the compression moves complexity elsewhere, and that's where I tripped.

---

## The journey at a glance

| Stage | Mistake | What it taught me |
|---|---|---|
| 1 | Wanting to use `select { case x := <-items: ... }` for the consumer | `range` is for "drain one thing until done"; `select` is for multiplexing |
| 2 | `go func() { ... }` without trailing `()` | `go` needs a function **call**, not a definition |
| 3 | `for item := range items { ... }` with `item` unused | Go's unused-variable rule applies inside `range` too — use `for range items` |
| 4 | Forgetting `produced.Add(1)` after sending | Channels don't do your counting for you |
| 5 | Typo `producer.Add(1)` vs `produced` | Read the compiler error; fix the exact line it names |

Short list because channels hide most of the traps the condvar / semaphore versions exposed. The mistakes here are about Go syntax and channel idioms, not algorithms.

---

## Mistake 1: reaching for `select` when `range` was right

### What I wanted to write
```go
for {
    select {
    case consumed_item := <-items:
        consumed.Add(1)
    }
}
```

### Why it's wrong (even if you add the `ok` check)

Two separate problems:

**1. Without the `ok` check, you loop forever after close.** Receiving from a closed channel **never blocks** — it instantly returns the zero value (`""` for `string`). The select always picks that case. The loop spins and counts forever.

**2. Even *with* `case item, ok := <-items:` + `if !ok { return }`, you're using an 8-line construct to express what `for range items { ... }` says in 4 lines.**

### The rule of thumb I now use

| Intent | Construct |
|---|---|
| "Drain this one channel until it closes." | `for x := range ch` |
| "Wait for any of these things, whichever comes first." | `select` |
| "Try to send/receive; give up if it would block." | `select` with `default:` |
| "Receive with a timeout." | `select` with `<-time.After(...)` or `<-ctx.Done()` |

The instinct to reach for `select` was coming from the barbershop scaffold — where the barber genuinely needs to multiplex over `chairs` and `shutdown`. That's a legit use. The producer-consumer consumer doesn't multiplex over anything, so `range` wins.

### The deeper idea
`range` says *"I'm going to drain this one thing until it's done."*
`select` says *"Something will happen on one of these; do whichever."*
Match the construct to the intent.

---

## Mistake 2: `go func() { ... }` without the trailing `()`

### What I wrote
```go
go func() {
    pwg.Wait()
    close(items)
}          // <-- no parens here
```

### The compiler error
> `go discards result of func()`
>
> (or equivalent: `go requires function call, not conversion`)

### Why it's wrong
`go EXPR` expects `EXPR` to be a **function call**. `func() { ... }` is a function *value* (a definition), not a call. The call is the `()` on the end.

Think of it as two steps that Go collapses:
1. **Define** an anonymous function: `func() { ... }` — this evaluates to a function value.
2. **Call** it immediately: `func() { ... }()` — the trailing `()` invokes it.

`go` runs step 2 in a new goroutine. Without step 2, there's nothing to run.

### The mental template
```go
go func() {
    // body
}()        // <-- ALWAYS two characters at the end
```

If you find yourself writing `go func() { ... }` and then pausing, you're probably missing the `()`. Train your fingers to type the parens immediately after the closing brace.

---

## Mistake 3: `item` declared but not used inside `range`

### What I wrote
```go
for item := range items {
    consumed.Add(1)
}
```

### The compiler error
> `item declared and not used`

### Why it's wrong
Go's unused-variable rule is global — it applies inside `range` bodies too. If I declare `item` but never reference it, the compiler refuses to build.

### The fix I used
```go
for range items {
    consumed.Add(1)
}
```

Ranging without a binding is legal and is exactly what you want when you only care *that* something arrived, not *what* arrived.

### The other valid fix
```go
for item := range items {
    _ = item          // explicit "I see it, I'm choosing to ignore it"
    consumed.Add(1)
}
```

Use the first form when the value truly doesn't matter. Use the second when you might come back and start using `item` soon, so you don't have to touch the `for` line again.

### The lesson
In Go, unused variables are errors, not warnings. That extends to loop bindings. The fix is almost always to drop the binding.

---

## Mistake 4: forgetting `produced.Add(1)` after the send

### What I had
```go
item := fmt.Sprintf("P%d#%d", pid, i)
items <- item
// (no counter bump)
```

### Why I missed it
Channels handle the **handoff**. But they don't handle the **accounting**. The send succeeded, the consumer received it, `consumed.Add(1)` ran — but `produced` never moved because I never told it to.

When I ran, the output was:
```
Produced=0 Consumed=150 Expected=150
panic: mismatch — lost or double-counted items
```

That's the giveaway: `Consumed` matched, `Produced` didn't. The counter that's wrong tells you where you forgot to count.

### The mental template
Every side-effect I care about tracking needs its own line:
```go
item := fmt.Sprintf("P%d#%d", pid, i)  // build
items <- item                           // handoff
produced.Add(1)                         // track
```

Three lines, three verbs, each one my responsibility.

### The lesson
Concurrency primitives give you correctness for **synchronization**, not **observability**. If you want to count or log, that's on you, and the counter sits *outside* the primitive.

---

## Mistake 5: typo `producer.Add(1)` vs `produced`

Trivial, but worth calling out the reflex: **the compiler told me the exact line and exact identifier** (`producer_consumer/channel/main.go:82:5: undefined: producer`). Read compiler errors literally. Don't skim.

---

## The big idea: a channel IS a bounded buffer

This is the one-line take from the whole exercise.

| Concept | C++ condvar version | Go channel version |
|---|---|---|
| Bounded buffer data structure | `std::deque<T>` inside a class | `make(chan T, N)` |
| "Wait while full" | `not_full_.wait(lk, [this] { return queue_.size() < capacity_ \|\| closed_; })` | `items <- x` (blocks) |
| "Wait while empty" | `not_empty_.wait(lk, [this] { return !queue_.empty() \|\| closed_; })` | `<-items` (blocks) |
| "Signal one waiter" | `not_empty_.notify_one()` | (automatic, on send) |
| "Signal everyone on shutdown" | `not_full_.notify_all(); not_empty_.notify_all()` | `close(items)` |
| "I'm done, go home" (consumer side) | check `closed_ && queue_.empty()` → `nullopt` | range loop exits |
| Lines of sync logic | ~90-line class | 3 operators + 1 closer goroutine |

The whole condvar apparatus (predicate, `while` loop, signal pairing, broadcast on shutdown) is absorbed into the runtime. What used to be a fiddly protocol between producer and consumer is now just "send" and "receive."

### What I gave up

1. **Explicit state to assert on.** No more `assert(queue_.size() <= capacity_)`. The buffer is opaque. Debugging moves from "inspect the invariant" to "read the stacks of stuck goroutines" (`SIGQUIT` dump or `pprof`).
2. **A safe shutdown return code.** In C++, `push` returned `false` if closed. In Go, sending on a closed channel **panics**. There's no graceful way to push after close. That's what forces the closer-goroutine pattern below.
3. **Per-thread wake granularity.** `notify_one` vs `notify_all` was my choice. With a channel, every receiver on it is effectively an `notify_one` — I can't ask to wake "all of them at once." Workarounds: close the channel (wakes all rangers once, cleanly, to exit) or use a `chan struct{}` and close it as a broadcast primitive.

---

## The close-coordination pattern

The one non-trivial thing I had to think about:

> **Only senders may close. Closing twice panics. Sending on a closed channel panics. With N producers, which one closes?**

### The answer
Not any individual producer. A dedicated **closer goroutine**.

```go
go func() {
    pwg.Wait()     // every producer has called pwg.Done()
    close(items)   // now-and-only-now, signal end-of-stream
}()
```

Why this is the right shape:
- No producer knows individually whether it's the last one. The WaitGroup counts for them.
- `pwg.Wait()` returns *exactly* when every producer's `defer pwg.Done()` has fired → all producers are past their last send.
- `close(items)` therefore can't race with any send.
- Consumers ranging over `items` observe the close, drain whatever's buffered, and exit.

### What not to do

1. **Close from inside a producer.**
   - "Only the producer with `i == ItemsPerProducer - 1` closes." → Two producers race, one closes while another is mid-send → `panic: send on closed channel`.
2. **Close from main.**
   - `main()` doesn't know when producers finished unless it already called `pwg.Wait()`. If you call `pwg.Wait()` and then `close()` synchronously from main, that works — but then you've just serialized main against every producer, and the close-goroutine pattern is cleaner because it keeps main free to also `cwg.Wait()`.
3. **Not close at all.**
   - Consumers range forever. `cwg.Wait()` hangs. Program never exits.

### The "close is a broadcast" trick
If you ever need a Go equivalent of "broadcast to N listeners so they all exit," the idiom is:
```go
done := make(chan struct{})
// ... N goroutines select on <-done ...
close(done)   // every blocked receive returns zero-value immediately
```
That's how you fake `notify_all` with channels.

---

## Trade-off table: three variants side by side

| Aspect | C++ condvar | C++ semaphore | Go channel |
|---|---|---|---|
| Lines of sync logic | ~90 | ~100 | ~20 |
| Predicate loop required | Yes | No | No |
| Spurious wakeups to handle | Yes | No | No |
| Shutdown primitive | `notify_all` | Manual N-permit releases | `close()` |
| Shutdown failure mode | Hang if you forget to broadcast | Hang / starve if wrong permit count | Hang if no closer; panic if wrong sender |
| Per-thread debugging | Predicate is printable | Permit balance opaque | Goroutine stacks via pprof/SIGQUIT |
| Matches lecture pseudocode | `wait`/`signal` style | `wait(sem)`/`signal(sem)` style | N/A (different paradigm) |
| Failure on misuse | Logic bug (silent) | Logic bug (silent) | Runtime panic (loud) |

"Failure on misuse" is interesting: Go is **louder** than C++ at runtime. Sending on a closed channel crashes immediately; signaling a stale condvar just silently breaks correctness. Go trades graceful error paths for visible-at-runtime diagnostics. That matches its overall design philosophy.

---

## Sabotage experiments to cement the lessons (30s each)

1. **Remove the closer goroutine.**
   Run → program hangs. `Ctrl-\` (SIGQUIT) dumps goroutine stacks; you'll see consumers stuck in `chan receive`. That's the "goroutine leak" bug `-race` can't catch but SIGQUIT can.

2. **Move `close(items)` inside a producer** (e.g., as the last statement of producer 0):
   Run → `panic: send on closed channel`. Producers 1 and 2 raced past the close.

3. **Call `close(items)` twice** (add a redundant close after `pwg.Wait()`):
   Run → `panic: close of closed channel`. close() is not idempotent.

4. **Remove `produced.Add(1)`** (undo Mistake 4):
   Run → `Produced=0 Consumed=150 Expected=150` + panic. Reminds you that accounting lives outside the primitive.

5. **Swap `for range items` for `for item := range items`** without using `item`:
   Run → compile error. Same class as Mistake 3.

Each one is a 5-second edit and a 3-second `go run`. The loop of sabotage → observe → revert is the fastest way to internalize the invariants.

---

## Checklist for the next Go channel-based sync problem

Before hitting run:
- [ ] Is the channel's capacity right for the "bounded buffer" part of the spec?
- [ ] Who are the senders? Who's allowed to `close()`?
- [ ] Is there exactly one site that calls `close()`? (Often: a dedicated goroutine `pwg.Wait(); close(ch)`.)
- [ ] Do I have a `go func() { ... }()` and not `go func() { ... }`?
- [ ] Is every `range` binding either used or dropped?
- [ ] Am I using `range` when I just drain, and `select` only for multiplexing / timeouts / cancellation?
- [ ] Do I have a `sync.WaitGroup` for every set of goroutines I need to join? (Usually two: senders + receivers.)
- [ ] Am I running with `-race`? (Yes. Always. It's free.)

---

## What this chapter added to my mental toolkit

1. **Channels move complexity.** They don't eliminate it — they shift it from steady-state (where condvars have predicates) to shutdown (where you have to think about who closes, and closing twice panics).
2. **`range` > `select` for single-channel drain.** Reach for `select` only when multiplexing.
3. **`go func()()` — both parens matter.** One defines, the other runs.
4. **The counter lives outside the primitive.** Channels synchronize; they don't observe. `.Add(1)` is still my job.
5. **Go panics at runtime where C++ silently misbehaves.** Both styles have a place; the Go style is easier to debug but terrifying to deploy without coverage.
6. **Close-as-broadcast (`close(done)` on a `chan struct{}`) is the Go substitute for `notify_all` when you need to wake N goroutines at once.** Remember this pattern for barbershop / barrier later.
