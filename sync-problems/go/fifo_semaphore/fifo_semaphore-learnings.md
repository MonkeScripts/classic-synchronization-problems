# FIFO Semaphore (Go): Mistakes & Learnings

A record of implementing the FIFO semaphore in Go using the **daemon-goroutine** strategy (lecture demo 7, mirrors the H2O daemon you already wrote). The daemon owns a counter and a queue of per-waiter channels, and serves arrivals strictly FIFO by construction.

Companion to `../h2o/h2o-learnings.md` — the H2O notes cover the broader "channel-of-channels" / client-server pattern. This doc focuses on what's new in fifo_semaphore: the **single-`select`-multi-case daemon shape** and the **two-branch invariant for release**.

---

## The headline observation

A FIFO semaphore is `count + queue-of-waiters`, and in Go the cleanest way to express that is to put both inside a daemon goroutine:

- `count int` — local variable in the goroutine (no mutex; only the daemon ever touches it).
- `waiters *list.List` — local variable in the goroutine (same).
- Two channels expose the API:
  - `acquireCh chan chan struct{}` — acquirers send their *private reply chan* in.
  - `releaseCh chan struct{}` — releasers signal "a permit is available."

The daemon's `for { select { ... } }` loop reacts to whichever channel is ready, atomically updating the local state. Because all mutable state lives in one goroutine, **there is literally no shared state to race over** — `-race` finds nothing.

This is the same client-server pattern you already practiced with H2O, applied to a different problem.

---

## The protocol

```go
type FifoSemaphore struct {
    acquireCh chan chan struct{}    // acquirers send a reply chan in
    releaseCh chan struct{}         // bare signal
}

func NewFifoSemaphore(initial int) *FifoSemaphore {
    s := &FifoSemaphore{
        acquireCh: make(chan chan struct{}),
        releaseCh: make(chan struct{}),
    }
    go func() {
        count := initial
        waiters := list.New()
        for {
            select {
            case <-s.releaseCh:
                if waiters.Len() > 0 {
                    e := waiters.Front()
                    waiters.Remove(e)
                    e.Value.(chan struct{}) <- struct{}{}    // hand permit to oldest waiter
                } else {
                    count++                                   // no waiter, bank the permit
                }
            case ch := <-s.acquireCh:
                if count > 0 {
                    count--
                    ch <- struct{}{}                          // permit available, wake immediately
                } else {
                    waiters.PushBack(ch)                      // queue the waiter
                }
            }
        }
    }()
    return s
}

func (s *FifoSemaphore) Acquire() {
    ch := make(chan struct{})
    s.acquireCh <- ch
    <-ch
}

func (s *FifoSemaphore) Release() {
    s.releaseCh <- struct{}{}
}
```

The structural rule that makes this correct: **a release does exactly one thing per call — wakes a waiter, or bumps count, never both.** Same two-branch invariant from the C++ queue version (`queue.cpp`).

---

## Mistakes I hit

### Syntax

#### 1. `select <-channel :` is not valid Go

```go
select <-fifoSem.releaseCh :       // ❌ parse error
    if waiters.Len() > 0 { ... }
```

Easy slip from C-style switch fall-through. Go's `select` is *always* a block:

```go
select {
case <-releaseCh:
    ...
case ch := <-acquireCh:
    ...
}
```

If you only have one channel to handle, you don't need `select` at all — just `<-channel`. But the daemon needs to handle *either* release *or* acquire whenever one is ready, which is exactly what `select` is for.

#### 2. `} else {` must be on one line

```go
}
else {     // ❌ parse error
```

Same rule as the rest of Go's brace style — the `else` (and `} else if`) must come on the same line as the closing brace of the prior `if`.

#### 3. `go func() { ... }` without `()` doesn't run

```go
go func() {
    // daemon body
}        // ❌ this just defines the literal; doesn't call it
```

Without the trailing `()`, you've written a function literal expression and used it as the operand of `go` — but `go` requires a *call*, not a definition. Need:

```go
go func() {
    // daemon body
}()      // ✅ invoke
```

This is the same pattern as IIFEs in JavaScript: `(function(){})()`. Easy to drop the parens when the function body is long.

### Types

#### 4. `releaseCh` declared as `chan chan struct{}` but used as `chan struct{}`

I started with the H2O pattern in mind (`chan chan struct{}` for both acquireCh and releaseCh) and only updated half the references. Compiler errors:

```
cannot use make(chan struct{}, 100) as chan chan struct{} value in assignment
cannot use struct{}{} as chan struct{} value in send
```

The asymmetry matters: **acquirers need to send their reply chan** (so the daemon has a way to wake exactly that acquirer), but **releasers don't need a reply** — they're just signaling "a permit is available." So:

```go
acquireCh chan chan struct{}    // chan-of-chan: pass reply line in
releaseCh chan struct{}         // bare signal: no reply needed
```

#### 5. Forgot to import `container/list`

The compiler error cascade was misleading — it reported `q.Init undefined`, `q.Front undefined`, etc., as if my embedded type was wrong. Actually it was because I never imported `container/list` and the embedded `list.List` couldn't resolve. One missing import, six downstream errors.

> **Lesson:** when you see a flood of `<X> undefined` errors after touching a single file, suspect a missing import before suspecting your type design.

### Logic / structure

#### 6. Two sequential `select`s instead of one with two cases

```go
for {
    select <-releaseCh: ...     // ❌ wait for release first
    select <-acquireCh: ...     // ❌ then wait for acquire, alternating
}
```

This would force the daemon to alternate strictly: release, then acquire, then release, then acquire. If three acquires arrive before any release, the second one blocks forever. The daemon must be willing to handle **either case whenever it's ready**, which means one `select` with two `case`s:

```go
for {
    select {
    case <-releaseCh:
        ...
    case ch := <-acquireCh:
        ...
    }
}
```

> **Lesson:** "two channels, react to whichever fires first" → one `select` with two `case`s. "Process A then process B then loop" → two separate operations in sequence. Pick the right shape for the temporal relationship you actually want.

#### 7. Missing `ch :=` bindings inside `case`s

```go
case <-releaseCh:
    ch := waiters.Pop()        // ✅ pop waiter from queue
    ch <- struct{}{}

case ch := <-acquireCh:        // ✅ bind the reply chan
    if count > 0 { ch <- struct{}{} ... }
```

In the release case, the channel comes from the queue (pop). In the acquire case, it comes from the channel itself (`case ch := <-acquireCh:`). Two different sources, both bound with `:=`.

#### 8. `fifo.releaseCh` — undefined name

Typo: I used `fifo` in `Release()` but the receiver is named `s`. Compiler caught it.

> **Lesson:** keep receiver names consistent across methods on the same type. Go convention is short (`s`, `f`, `wf`) but it has to be the same name everywhere.

---

## Conceptual: why no mutex is needed

This is the deepest property of the daemon design and worth naming.

`count` and `waiters` are **local variables inside the daemon goroutine**. No other goroutine has a reference to them. The only way to influence them is to send on `acquireCh` or `releaseCh`, and the daemon serializes those sends through its own `select` loop.

> **Single-goroutine ownership of mutable state ⇒ no synchronization primitives required.** The channels are the synchronization.

The Go race detector confirms this: `go run -race .` finds zero data races, despite no `sync.Mutex` anywhere in the code. Compare to the cv-based C++ version where the mutex is essential — different language, different idiom for the same problem.

---

## On buffer sizes — "why is this buffered?"

I started with `make(chan chan struct{}, 100)` and asked myself the right question: do I need the buffer?

Answer: **no, not strictly.** Both channels can be unbuffered:

```go
acquireCh: make(chan chan struct{})    // unbuffered = rendezvous
releaseCh: make(chan struct{})         // unbuffered = rendezvous
```

With unbuffered channels, the caller of `Acquire()` or `Release()` blocks until the daemon receives. Since the daemon's per-case work is microseconds (a counter increment or a queue push), the wait is invisible.

A buffer would matter only if many acquirers/releasers arrive simultaneously and the daemon can't drain fast enough to keep up. For our 16-thread test, that doesn't happen.

> **Default to unbuffered.** Add a buffer only when you have evidence that callers shouldn't block on the daemon — and even then, prefer the smallest buffer that does the job. Cargo-culted large buffers can mask real backpressure problems.

---

## Summary: the Go FIFO-semaphore muscle memory

### Syntax
1. **`select` is always a block: `select { case ...: ... }`.** No bare-channel `select <- ch:` form.
2. **`go func() { ... }()` — don't drop the trailing `()`.** Without it, you've just defined a function literal.
3. **`} else {` on one line.** Same brace rule as the rest of Go.
4. **Receiver names are consistent across methods.** Pick `s` and use `s` everywhere.

### Channels
5. **Asymmetric channel types are normal.** `acquireCh chan chan struct{}` (need reply addressed to a specific sender) vs `releaseCh chan struct{}` (bare broadcast-style signal). Different jobs, different types.
6. **Default to unbuffered.** Add a buffer only when you can articulate which case it solves.

### Daemon design
7. **Single `select` with multiple `case`s for "react to whichever fires first."** Two sequential `select`s force alternation, which is almost never what you want.
8. **Single-goroutine ownership of mutable state ⇒ no mutex.** The channels do the synchronization. `-race` will confirm.
9. **Two-branch invariant for `release`:** wakes a waiter **or** bumps count, never both. Same as the C++ queue version, same as any dual-mode resource grant.

### Cross-cutting (vs C++)
10. **Same problem, very different idioms.** C++ ticket queue uses atomics + cv; C++ queue version uses mutex + per-waiter binary_semaphore; Go daemon uses a goroutine + channels. The lecture's three demos map to three primitives, but the *invariant* (release-emits-exactly-one-permit) is identical across all of them.
11. **Lost-wakeup hazards don't exist in the daemon design** because there's no cv to notify-into-a-void. The channel send `ch <- struct{}{}` rendezvous with the waiter's receive — Go channels have memory in the buffered case and are atomic in the unbuffered case.
