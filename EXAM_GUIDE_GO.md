# CS3211 — Go Exam Book

> **How to use this book during the exam.** Stay in Go headspace. §1 is the cheat sheet. §2 is the centerpiece — full **program scaffolding** shapes you can write from blank, plus the channel/goroutine idioms that go inside them. §3 collects classical problems already written in Go. §4 walks the 2025-style scenarios (LoadBalancer Q27, graceful shutdown Q28). §5–§7 are pitfalls, build commands, and a decision matrix.
>
> If a question gives you a half-skeleton, jump to §4. If it says "implement X using channels and goroutines" from scratch, start at §2.

---

## Table of contents

0. [Keyword index — scenario phrase → section](#0-keyword-index)
1. [Primitive cheat sheet](#1-primitive-cheat-sheet)
2. [Idioms & scaffolding](#2-idioms--scaffolding) ← **centerpiece**
   - 2a. [Program scaffolding](#2a-program-scaffolding) (S1–S6)
   - 2b. [Sync idioms](#2b-sync-idioms) (I1–I14)
3. [Classical problems in Go](#3-classical-problems-in-go)
4. [Exam-style scenarios](#4-exam-style-scenarios)
5. [Pitfalls catalogue](#5-pitfalls-catalogue)
6. [Build / run / debug](#6-build--run--debug)
7. [Decision matrix](#7-decision-matrix)

---

## 0. Keyword index

| If the question mentions… | Look at |
|---|---|
| "use channels and goroutines" / "no sync package" | §2a S6 (channels-only struct), §4.1 (LoadBalancer) |
| dispatch / load balance / route to one of N | §2b I12 (chan-of-chan), §4.1 |
| graceful shutdown / drain / no requests left unprocessed | §2a S5, §2b I6, §4.2 |
| W workers / SQ-CQ / pipeline / fan-out / fan-in | §4.3 (channel server) |
| STM / transaction / atomic block / read-set / write-set | §4.4 (STM Go API), §4.5 (OCC coordinator from A2) |
| OCC / snapshot / commit-validate / no sync allowed | §4.5 (OCC from A2) |
| bounded buffer / producer / consumer / queue | §2b I3, §3.1 |
| readers / writers | §3.2 |
| barrier / round | §2b I11, §3.3 (NOT `sync.WaitGroup` — single-use!) |
| philosophers / chopsticks / forks | §3.4 |
| barber / chairs / balking / waiting room | §3.5 |
| H2/O / molecules | §3.6 (daemon strategy) |
| FIFO / fairness | §3.7 (daemon strategy) |
| three-role / search-insert-delete | §3.8 |

---

## 1. Primitive cheat sheet

| Primitive | Package | What it gives you |
|---|---|---|
| `chan T` (unbuffered, cap 0) | builtin | **Rendezvous**: send blocks until paired recv. NOT a semaphore. |
| `chan T` (buffered cap N) | builtin | Tiny bounded queue. |
| `chan struct{}` cap 1 | builtin | Binary semaphore / mutex (Pattern B is idiomatic). |
| `chan struct{}` cap N | builtin | Counting semaphore. |
| `close(ch)` on `chan struct{}` | builtin | Broadcast — every recv returns immediately (zero value). |
| `chan chan T` | builtin | Channel of channels — used for daemon mailboxes (each request carries its reply line). |
| `select { ... }` | builtin | Multiplex over channel ops. With `default:` → non-blocking. |
| `sync.Mutex` | `sync` | Mutual exclusion. **Not reentrant.** |
| `sync.RWMutex` | `sync` | Reader-writer lock with writer preference. |
| `sync.WaitGroup` | `sync` | **Single-use latch**, NOT a cyclic barrier. |
| `sync.Cond` | `sync` | Parking on a predicate. Pair with mutex. |
| `sync.Once` | `sync` | Run-exactly-once initialization. |
| `atomic.Int32`, `atomic.Bool` | `sync/atomic` | Per-op atomicity. |
| `time.Sleep`, `time.After`, `time.NewTicker` | `time` | Delays, timeouts, periodic firing. |
| `signal.Notify` | `os/signal` | Receive OS signals (e.g. SIGINT) on a channel. |
| `context.Context` | `context` | Request-scoped cancellation; `ctx.Done()` is `<-chan struct{}`. |

---

## 2. Idioms & scaffolding

This is the heart of the book. **§2a teaches you to write a working concurrent Go program from a blank file.** §2b shows the channel/goroutine idioms that go inside §2a's shapes. Every classical problem (§3) and exam scenario (§4) is just §2a + §2b composed.

### 2a. Program scaffolding

#### S1. The standard "main + goroutines + WaitGroup" shape

The skeleton every Go concurrency answer slots into. Memorize the import block and the `for + go func(){ defer wg.Done(); ... }() + wg.Wait()` shape.

```go
package main

import (
    "fmt"
    "sync"
    "sync/atomic"
    "time"
)

const (
    NWorkers = 4
    Rounds   = 100
)

func main() {
    var wg sync.WaitGroup
    var counter atomic.Int32

    for i := 0; i < NWorkers; i++ {
        wg.Add(1)                                 // *** before the go statement ***
        go func(id int) {                         // capture id by VALUE, not by ref
            defer wg.Done()
            for r := 0; r < Rounds; r++ {
                counter.Add(1)
                time.Sleep(time.Millisecond)
            }
        }(i)                                      // *** call, not just definition ***
    }

    wg.Wait()
    fmt.Println("counter =", counter.Load())
}
```

**Critical invariants:**
- **`wg.Add(1)` happens BEFORE `go`,** never inside the goroutine. If `Add` runs after the parent returns from spawn, `Wait` may have already fired with count 0.
- **Capture loop variable by value: `go func(id int) { ... }(i)`.** Capturing `i` by reference means every goroutine sees the final loop value (Go ≤ 1.21 — fixed in 1.22, but write the safe form).
- **`go func(){...}()` not `go func(){...}`.** `go` requires a **call expression**, not a function definition.
- **`go func()` body must end with the call's parentheses.** Forgetting `()` is a syntax error or worse — silently scheduled `<nothing>`.

#### S2. Struct + constructor returning `*T`

The shape every "implement a thread-safe X" question reduces to. Fields go in the struct; a `New*` function initializes them; methods are `func (x *T) ...`.

```go
type Buffer struct {
    items   chan string                            // channel as a bounded queue
    closed  chan struct{}                          // close-broadcast signal
    closeOnce sync.Once                            // ensure Close() is idempotent
}

func NewBuffer(cap int) *Buffer {
    return &Buffer{
        items:  make(chan string, cap),            // *** explicit make in constructor ***
        closed: make(chan struct{}),
    }
}

func (b *Buffer) Push(x string) (ok bool) {
    select {
    case <-b.closed:
        return false                                // closed
    case b.items <- x:
        return true                                 // sent
    }
}

func (b *Buffer) Pop() (x string, ok bool) {
    select {
    case <-b.closed:
        // drain whatever's left, then signal closed
        select {
        case x = <-b.items:
            return x, true
        default:
            return "", false
        }
    case x = <-b.items:
        return x, true
    }
}

func (b *Buffer) Close() {
    b.closeOnce.Do(func() { close(b.closed) })     // panic-safe — idempotent
}
```

**Shape rules to memorize:**
- **`New*` functions return `*T` (pointer).** Channels and mutexes can't be safely copied — pointer return makes that the canonical form.
- **`make()` channel fields in the constructor.** `&Buffer{}` creates a struct with **nil channels**. A nil channel blocks sends/recvs forever — silent deadlock. Force callers through the constructor.
- **`Close()` is idempotent via `sync.Once`** when channels are involved. `close(ch)` twice panics. Wrapping in `sync.Once.Do` makes it safe to call from anywhere.

#### S3. Multiple variants in one package (typed by name)

When a question asks "try strategy X and strategy Y," declare separate types or use a type-switch. Each has its own constructor.

```go
type DaemonFactory struct  { /* daemon strategy fields */ }
type LeaderFactory struct  { /* leader strategy fields */ }

func NewDaemonFactory() *DaemonFactory { /* ... */ }
func NewLeaderFactory() *LeaderFactory { /* ... */ }

// shared interface
type Factory interface {
    Hydrogen(id int)
    Oxygen(id int)
}

func main() {
    for _, f := range []Factory{NewDaemonFactory(), NewLeaderFactory()} {
        runStrategy(f)
    }
}
```

#### S4. Daemon goroutine pattern (the Go answer to "centralized coordinator")

Pattern: a long-running goroutine owns mutable state; clients communicate via channels in the surrounding struct. **Single-goroutine ownership of state means no mutex is needed** — channels do the synchronization.

```go
type Service struct {
    req chan request                                // public mailbox
}

type request struct {
    payload string
    reply   chan struct{}                           // each request carries its reply line
}

func NewService() *Service {
    s := &Service{req: make(chan request)}
    go s.daemon()                                   // *** SPAWN at construction ***
    return s
}

func (s *Service) daemon() {
    state := 0                                       // local — no mutex needed
    for r := range s.req {                           // ranges until s.req is closed
        // process r.payload, mutate `state`
        state++
        r.reply <- struct{}{}                        // wake the requester
    }
    // post-shutdown cleanup if needed
}

func (s *Service) Call(payload string) {
    reply := make(chan struct{})
    s.req <- request{payload: payload, reply: reply}
    <-reply                                          // wait for daemon's ack
}

func (s *Service) Stop() { close(s.req) }            // daemon exits when range drains
```

**Critical:**
- **Spawn the daemon in the constructor.** A struct holding channels is not a service until something reads them.
- **Local state is single-owner.** Don't expose it; channels are the only API.
- **Close the inbound channel to shut down.** The daemon's `range` exits cleanly. Any pending requesters are stranded — drain them or document the contract.

#### S5. Graceful shutdown with `signal.Notify` + done channel

Pattern for "Ctrl-C drains all in-flight work, then exits cleanly."

```go
package main

import (
    "context"
    "fmt"
    "os"
    "os/signal"
    "sync"
    "syscall"
)

func main() {
    ctx, cancel := context.WithCancel(context.Background())
    defer cancel()                                    // *** always defer cancel ***

    // wire SIGINT/SIGTERM into ctx
    sigCh := make(chan os.Signal, 1)
    signal.Notify(sigCh, syscall.SIGINT, syscall.SIGTERM)
    go func() {
        <-sigCh                                       // first signal
        fmt.Println("shutting down...")
        cancel()                                      // broadcast cancellation
    }()

    var wg sync.WaitGroup
    for i := 0; i < 4; i++ {
        wg.Add(1)
        go worker(ctx, i, &wg)
    }
    wg.Wait()
    fmt.Println("clean exit")
}

func worker(ctx context.Context, id int, wg *sync.WaitGroup) {
    defer wg.Done()
    for {
        select {
        case <-ctx.Done():                            // cancellation propagates here
            return
        default:
            // do one unit of work
        }
    }
}
```

**Why `context.Context` and not just a `done` channel.** `context.Context` already provides `Done() <-chan struct{}`, so you get the close-broadcast pattern for free, plus deadline/value propagation if you need it. For exam answers either is fine; just pick one and be consistent.

#### S6. Channels-only struct (Q27 constraint: NO `sync` package)

When the question forbids `sync.Mutex`/`sync.WaitGroup`, model exclusion and counting with channels only.

```go
type Server struct {
    requests chan Request                            // dispatch mailbox
    quit     chan struct{}                           // close-broadcast for shutdown
    done     chan struct{}                           // signal "I'm fully drained"
}

type Request struct {
    UserID, SeatID int
    Reply          chan bool                         // reply line — like S4
}

func NewServer(cap int) *Server {
    s := &Server{
        requests: make(chan Request, cap),
        quit:     make(chan struct{}),
        done:     make(chan struct{}),
    }
    go s.run()
    return s
}

func (s *Server) run() {
    defer close(s.done)
    for {
        select {
        case <-s.quit:
            // drain any pending requests
            for {
                select {
                case r := <-s.requests:
                    s.handle(r)
                default:
                    return
                }
            }
        case r := <-s.requests:
            s.handle(r)
        }
    }
}

func (s *Server) Stop() {
    close(s.quit)                                    // broadcast: stop accepting new
    <-s.done                                         // wait for full drain
}
```

**Why this works without sync.** Each `Server` is a single goroutine reading the `requests` channel — that goroutine has exclusive access to all per-server state. Counting "in flight" is implicit in channel buffer occupancy. Shutdown is `close(quit)` + `<-done`. Same shape as S4 + S5.

---

### 2b. Sync idioms

These are the snippets that fill in the shapes from §2a. Each idiom is self-contained — no flipping to other files.

#### I1. `chan struct{}` cap 1 — Pattern B (mutex / binary semaphore)

**When.** Default mutex / binary-semaphore shape. "I put myself in, I take myself out."

```go
sem := make(chan struct{}, 1)                       // *** cap 1, NOT cap 0 ***
sem <- struct{}{}                                    // acquire (blocks if full)
defer func() { <-sem }()                             // release
```

**Gotchas.**
- **Cap 0 is RENDEZVOUS, not a semaphore.** `ch <- struct{}{}` with cap 0 blocks until a paired recv. For mutex semantics you need cap 1 (state lives in the buffer).
- Pattern B scales to cap N without rewriting (just change the `make` cap) — that's why it's idiomatic over Pattern A (token-as-resource).

#### I2. `chan struct{}` cap N — counting semaphore

**When.** Resource pool, rate limit, footman cap.

```go
sem := make(chan struct{}, N)                        // capacity = N permits
sem <- struct{}{}                                    // acquire
defer func() { <-sem }()                             // release
```

Mechanically identical to I1 with a different cap. The buffer occupancy IS the permit count.

#### I3. Bounded buffer (`chan T` cap N) — the producer-consumer collapse

**When.** Producer-consumer. **A channel literally IS a bounded buffer.**

```go
items := make(chan T, BufferSize)
items <- x                                           // blocks if full → backpressure
x := <-items                                         // blocks if empty
```

The whole condvar/semaphore producer-consumer apparatus collapses to two channel ops.

**Gotchas.**
- `close(items)` is the substitute for "shutdown broadcast" — every blocked recv returns the zero value with `ok=false`.
- **Sending on a closed channel panics. Closing a closed channel panics.** See I5 (closer goroutine) for the safe pattern.

#### I4. Drain until close — `for range`

**When.** Consumer side. The canonical "process each item until producer is done" loop.

```go
for x := range items {
    process(x)
}
// or, if x is unused:
for range items {
    counter.Add(1)
}
```

**Gotchas.**
- **Unused loop variable is a compile error.** Drop the binding (`for range items`).
- `range` on a channel exits cleanly when the channel is closed. Pair with I5.

#### I5. Closer goroutine pattern (one-site-closes)

**When.** Multiple producers; need exactly one site to close, after all producers are done.

```go
items := make(chan T, BufferSize)
var pwg sync.WaitGroup

for p := 0; p < NumProducers; p++ {
    pwg.Add(1)
    go func() {
        defer pwg.Done()
        // produce
    }()
}

go func() {
    pwg.Wait()
    close(items)                                     // *** the ONLY close site ***
}()

for x := range items {                               // consumer drains
    process(x)
}
```

**Gotchas.**
- **Only senders may close.** Closing from a non-sender races with sends.
- **Closing twice panics.** A dedicated closer goroutine guarantees one close site.
- **Don't close from inside a producer.** Race with other producers' sends. The closer goroutine waits on the producers' WG, then closes.

#### I6. Broadcast cancellation — `close(done)`

**When.** Wake N goroutines simultaneously (Go's substitute for `cv.Broadcast()`).

```go
done := make(chan struct{})

for i := 0; i < N; i++ {
    go worker(done)
}
close(done)                                          // EVERY blocked <-done returns
```

In the worker:
```go
func worker(done <-chan struct{}) {
    for {
        select {
        case <-done:
            return
        case x := <-work:
            process(x)
        }
    }
}
```

**Gotchas.**
- `done` is `chan struct{}` — payload irrelevant; the close itself is the signal.
- This is THE Go idiom for cancellation. `context.Context.Done()` is the same pattern wrapped.

#### I7. Multiplex with `select`

```go
select {
case x := <-ch1:                                     // arrived from ch1
case ch2 <- y:                                        // sent on ch2
case <-time.After(5 * time.Second):                  // timeout
case <-ctx.Done():                                    // cancellation
}
```

**Gotchas.**
- Without `default:`, blocks until at least one case fires.
- **Multiple ready cases → pseudo-random choice (NOT source order).** Don't rely on case ordering for priority.
- `nil` channels in a select case **never fire** — useful for dynamically disabling cases.

#### I8. Non-blocking try — `default:`

```go
select {
case ch <- x:                                         // sent immediately
default:                                              // would have blocked → fall through
}
```

**Gotchas.**
- `default:` (no `case` keyword!) is the actual non-blocking knob. `for { ... }` doesn't make anything non-blocking.
- This is the canonical "try-send" pattern for balking (see §3.5 barbershop, §4.1 LoadBalancer).

#### I9. Long-running worker loop with shutdown

```go
for {
    select {
    case <-shutdown:
        return                                        // exit arm — return is correct here
    case x := <-work:
        process(x)                                    // *** work arm — NO return ***
    }
}
```

**Gotchas.**
- **`return` in a work arm exits the FUNCTION, not the case.** Worker dies after one job.
- `for { select { ... } }` where every case `return`s — loop never iterates. Drop the `for`.

#### I10. Labeled break (escape `for` from inside `select`)

**When.** Need to exit a `for` from inside a nested `select` (try-and-back-off).

```go
acquire:
for {
    <-leftCh
    select {
    case <-rightCh:
        break acquire                                 // *** breaks the LOOP ***
    default:
        leftCh <- struct{}{}                          // give back, retry
    }
}
```

**Gotchas.**
- Bare `break` only exits the current select case (continues the for). Need a label.
- Same syntax for `continue acquire`.

#### I11. Cyclic barrier — `sync.Cond` + generation counter

**When.** Reusable phase synchronization. **NOT `sync.WaitGroup`** (single-use latch — reuse panics).

```go
type B struct {
    expected, count int
    generation      uint64
    mu              sync.Mutex
    cv              *sync.Cond
}

func NewB(n int) *B {
    b := &B{expected: n}
    b.cv = sync.NewCond(&b.mu)
    return b
}

func (b *B) ArriveAndWait() {
    b.mu.Lock()
    defer b.mu.Unlock()
    gen := b.generation
    b.count++
    if b.count == b.expected {
        b.count = 0
        b.generation++                                // *** advance before broadcast ***
        b.cv.Broadcast()
        return
    }
    for b.generation == gen {                         // explicit predicate loop
        b.cv.Wait()
    }
}
```

**Gotchas.**
- **Generation counter, NOT a `bool ready`.** A boolean can't tell round R from round R+1.
- **`sync.WaitGroup` reused** across rounds panics: *"WaitGroup is reused before previous Wait has returned."* It's a single-use latch.
- `sync.Cond.Wait()` releases `mu` while parked, re-acquires on wake. Same semantics as C++ `cv.wait`.

#### I12. Daemon goroutine + chan-of-chan (centralized protocol)

**When.** Centralized protocol orchestration where atoms / clients precommit by depositing a reply line.

```go
type Service struct {
    req chan chan struct{}                           // mailbox: each request carries its reply
}

func New() *Service {
    s := &Service{req: make(chan chan struct{})}
    go func() {
        for {
            reply := <-s.req
            // do work
            reply <- struct{}{}                       // wake the requester
        }
    }()
    return s
}

func (s *Service) Call() {
    reply := make(chan struct{})
    s.req <- reply                                    // precommit: deposit reply line
    <-reply                                           // commit: wait for go signal
}
```

**Multi-turn protocol** (e.g. H2O daemon needs go-then-done): reuse the same `reply` channel for both turns.
```go
reply <- struct{}{}                                   // daemon sends "go"
<-reply                                                // daemon waits for "done"
```

**Gotchas.**
- **Must `go`-spawn the daemon.** A struct holding channels is not a service.
- **Single-goroutine ownership of state ⇒ no mutex needed.** Channels are the synchronization.
- Daemon `for {}` runs forever — wire shutdown explicitly (close the inbound channel and `range`, or add a `quit` case).

#### I13. `sync.RWMutex` baseline

```go
var mu sync.RWMutex
var data map[string]string

// reader
mu.RLock()
v := data[k]
mu.RUnlock()

// writer
mu.Lock()
data[k] = v
mu.Unlock()
```

**Gotchas.**
- Go's RWMutex has writer preference since 1.x — won't starve writers under reader load.
- Use `defer mu.RUnlock()` if there's any chance of panic between RLock and RUnlock.

#### I14. `signal.Notify` + cancel context — graceful Ctrl-C

```go
sigCh := make(chan os.Signal, 1)                     // *** buffered ≥ 1 ***
signal.Notify(sigCh, syscall.SIGINT, syscall.SIGTERM)
go func() { <-sigCh; cancel() }()
```

**Gotchas.**
- **`make(chan os.Signal, 1)` MUST be buffered.** `signal.Notify` does a non-blocking send; an unbuffered channel with no immediate reader drops the signal.
- Always use `defer cancel()` after `context.WithCancel` to release resources even on the success path.

---

## 3. Classical problems in Go

### 3.1 Producer–consumer

**Scenario.** N producers, M consumers, channel of capacity K. Clean shutdown after every produced item is consumed.

**Invariants.** `produced == consumed` after shutdown.

#### Implementation — channel as bounded buffer + closer goroutine

Composes **S1** + **I3** + **I4** + **I5**.

```go
const (
    BufferSize       = 8
    NumProducers     = 3
    NumConsumers     = 2
    ItemsPerProducer = 50
)

func main() {
    items := make(chan string, BufferSize)
    var produced, consumed atomic.Int32
    var pwg, cwg sync.WaitGroup

    for p := 0; p < NumProducers; p++ {
        pwg.Add(1)
        go func(pid int) {
            defer pwg.Done()
            for i := 0; i < ItemsPerProducer; i++ {
                items <- fmt.Sprintf("P%d#%d", pid, i)   // blocks if full
                produced.Add(1)                           // counter is YOUR job
            }
        }(p)
    }

    for c := 0; c < NumConsumers; c++ {
        cwg.Add(1)
        go func() {
            defer cwg.Done()
            for range items {                             // drains until closed
                consumed.Add(1)
            }
        }()
    }

    go func() { pwg.Wait(); close(items) }()              // closer goroutine

    cwg.Wait()
    fmt.Printf("Produced=%d Consumed=%d Expected=%d\n",
        produced.Load(), consumed.Load(), NumProducers*ItemsPerProducer)
}
```

**Mistakes worth memorizing.**
1. Closing from a producer — race with another producer's send → `panic: send on closed channel`.
2. Forgetting `produced.Add(1)` — channels synchronize, NOT observe. Counters live OUTSIDE the primitive.
3. `for item := range items` with `item` unused — Go's unused-variable rule is a compile error. Drop the binding.

---

### 3.2 Readers–writers

**Scenario.** Many concurrent readers, occasional writer. Three increasingly sophisticated solutions.

#### Variant A — `sync.RWMutex` baseline

Composes **I13**.

```go
type KVCache struct {
    mu   sync.RWMutex
    data map[string]string
}

func (c *KVCache) Get(k string) string {
    c.mu.RLock(); defer c.mu.RUnlock()
    return c.data[k]
}
func (c *KVCache) Set(k, v string) {
    c.mu.Lock(); defer c.mu.Unlock()
    c.data[k] = v
}
```

#### Variant B — hand-rolled Lightswitch (channels-as-semaphore)

```go
type KVCache struct {
    data      map[string]string
    bibo      chan struct{}                              // cap 1, mutex over rc
    emptyRoom chan struct{}                              // cap 1
    rc        int                                        // PLAIN int — guarded by bibo
}

func NewKVCache() *KVCache {
    return &KVCache{
        data:      make(map[string]string),
        bibo:      make(chan struct{}, 1),               // *** cap 1, NOT cap 0 ***
        emptyRoom: make(chan struct{}, 1),
    }
}

func (c *KVCache) Get(k string) string {
    c.bibo <- struct{}{}                                 // acquire bibo
    c.rc++
    if c.rc == 1 { c.emptyRoom <- struct{}{} }           // first reader
    <-c.bibo
    v := c.data[k]
    c.bibo <- struct{}{}
    c.rc--
    if c.rc == 0 { <-c.emptyRoom }                        // last reader
    <-c.bibo
    return v
}

func (c *KVCache) Set(k, v string) {
    c.emptyRoom <- struct{}{}
    c.data[k] = v
    <-c.emptyRoom
}
```

#### Variant C — no-starve turnstile

Add `turnstile chan struct{}` (cap 1). Writers `turnstile <- struct{}{}` before `emptyRoom <- struct{}{}` and `<-turnstile` after `<-emptyRoom`. Readers do an immediate gate-pass (`turnstile <- struct{}{}; <-turnstile`) as their first action.

**Mistakes worth memorizing.**
1. `make(chan struct{})` (cap 0) for binary semaphore — that's RENDEZVOUS. Need cap 1.
2. Field access without receiver: `bibo <- struct{}{}` — Go has no implicit `this`. Always `c.bibo`.
3. Compound-op race on `c.rc` — but in this version `c.rc` is guarded by `c.bibo`, so it's fine. The rule is: any field that can be read or written outside the lock needs to be atomic.

---

### 3.3 Barrier

**Scenario.** N goroutines must reach a point before any may proceed. Reusable.

**Invariant.** No goroutine enters round R+1 while another is in round R.

#### Implementation — `sync.Cond` + generation counter

Composes **S2** + **I11**.

```go
type Barrier struct {
    expected, count int
    generation      uint64
    mu              sync.Mutex
    cv              *sync.Cond
}

func NewBarrier(n int) *Barrier {
    b := &Barrier{expected: n}
    b.cv = sync.NewCond(&b.mu)
    return b
}

func (b *Barrier) ArriveAndWait() {
    b.mu.Lock()
    defer b.mu.Unlock()
    gen := b.generation
    b.count++
    if b.count == b.expected {
        b.count = 0
        b.generation++
        b.cv.Broadcast()
        return
    }
    for b.generation == gen { b.cv.Wait() }
}
```

**Mistakes worth memorizing.**
1. Reusing one `sync.WaitGroup` (`Done; Wait; Add`) across rounds — panic: *"WaitGroup is reused before previous Wait has returned."* WG is a single-use latch.
2. Field as `sync.WaitGroup` (value) — carries `noCopy` lint. Use `*sync.WaitGroup` if you must.
3. `wg.Add(N)` inside a struct literal — `Add` returns nothing; literals take values. Use a constructor.

---

### 3.4 Dining philosophers

**Scenario.** N philosophers around a table, N chopsticks; each needs both neighbors' chopsticks.

#### Strategy A — channels as chopstick tokens (Pattern A)

Composes **I1** with the token-as-resource pattern.

```go
const N = 5

func main() {
    chopsticks := make([]chan struct{}, N)
    for i := range chopsticks {
        chopsticks[i] = make(chan struct{}, 1)            // cap 1, prime with token
        chopsticks[i] <- struct{}{}                        // *** PRIME at init ***
    }

    var wg sync.WaitGroup
    for i := 0; i < N; i++ {
        wg.Add(1)
        go func(pid int) {
            defer wg.Done()
            left, right := pid, (pid+1)%N
            // Asymmetric: last philosopher takes right first → break the cycle
            if pid == N-1 { left, right = right, left }
            for r := 0; r < Rounds; r++ {
                <-chopsticks[left]                          // take left
                <-chopsticks[right]                         // take right
                // EATING
                chopsticks[right] <- struct{}{}
                chopsticks[left] <- struct{}{}
            }
        }(i)
    }
    wg.Wait()
}
```

#### Strategy B — try-and-back-off with labeled break

Composes **I8** + **I10**.

```go
acquire:
for {
    <-chopsticks[left]
    select {
    case <-chopsticks[right]:
        break acquire                                       // got both
    default:
        chopsticks[left] <- struct{}{}                      // give left back
        time.Sleep(time.Microsecond)                        // brief backoff
    }
}
```

**Mistakes worth memorizing.**
1. `make(chan struct{}, 1)` without priming → first `<-chopsticks[i]` blocks forever.
2. All philosophers grab left first → deadlock cycle. Asymmetric ring is the simplest fix.

---

### 3.5 Barbershop

**Scenario.** One barber, K chairs in waiting room. Customer balks if all chairs full.

#### Implementation — channels with buffered chairs

Composes **S2** + **I7** + **I8**.

```go
type Barbershop struct {
    chairs       chan int                                  // BUFFERED cap CHAIRS
    barberReady  chan struct{}                             // unbuffered — rendezvous
    customerDone chan struct{}
    barberDone   chan struct{}
    shutdown     chan struct{}
}

func NewBarbershop(chairs int) *Barbershop {
    return &Barbershop{
        chairs:       make(chan int, chairs),
        barberReady:  make(chan struct{}),
        customerDone: make(chan struct{}),
        barberDone:   make(chan struct{}),
        shutdown:     make(chan struct{}),
    }
}

func (bs *Barbershop) Customer(id int) bool {
    select {
    case <-bs.shutdown:
        return false
    case bs.chairs <- id:                                  // sit
        <-bs.barberReady                                   // wait for "your turn"
        time.Sleep(2 * time.Millisecond)                   // haircut
        <-bs.barberDone                                    // wait for "leave"
        bs.customerDone <- struct{}{}                       // "I'm done"
        return true
    default:
        return false                                        // BALK
    }
}

func (bs *Barbershop) Barber() {
    for {
        select {
        case <-bs.shutdown:
            return
        case <-bs.chairs:
            bs.barberReady <- struct{}{}                    // "your turn"
            time.Sleep(2 * time.Millisecond)                // cut
            bs.barberDone <- struct{}{}                     // "you can leave"
            <-bs.customerDone                               // wait "I'm done"
        }
    }
}
```

**Mistakes worth memorizing.**
1. `case default:` — `default` is a keyword, no `case` precedes it. Bare `default:`.
2. Barber `return` after one customer in cut case — `return` exits the whole function. Long-running workers' work-arms must NOT return.
3. Both sides start with a `send` on unbuffered channel → mutual deadlock. Mirror the directions: one side sends-then-recvs, the other recvs-then-sends.

---

### 3.6 H₂O (water factory)

**Scenario.** Stream of H and O atoms; bond into water (2H + 1O per molecule).

#### Daemon strategy — chan-of-chan + central manager

Composes **S4** + **I12**.

```go
type WaterFactory struct {
    RequestH chan chan struct{}
    RequestO chan chan struct{}
}

func NewWaterFactory() *WaterFactory {
    wf := &WaterFactory{
        RequestH: make(chan chan struct{}),
        RequestO: make(chan chan struct{}),
    }
    go centralManager(wf)
    return wf
}

func centralManager(wf *WaterFactory) {
    for {
        h1 := <-wf.RequestH
        h2 := <-wf.RequestH
        o  := <-wf.RequestO
        h1 <- struct{}{}                                   // signal "go" to all 3
        h2 <- struct{}{}
        o  <- struct{}{}
        <-h1                                                // wait "done" — completion barrier
        <-h2
        <-o
    }
}

func (wf *WaterFactory) hydrogen(id int) {
    commit := make(chan struct{})
    wf.RequestH <- commit                                   // precommit
    <-commit                                                // commit (go)
    bondH(id)
    commit <- struct{}{}                                    // postcommit (done)
}

func (wf *WaterFactory) oxygen(id int) {
    commit := make(chan struct{})
    wf.RequestO <- commit
    <-commit
    bondO(id)
    commit <- struct{}{}
}
```

**The protocol is a 3-phase commit:** precommit (collect 2H + 1O), commit (signal all three to bond), postcommit (each atom sends `done`, daemon waits — completion barrier).

**Mistakes worth memorizing.**
1. Forgetting to spawn the daemon goroutine in the constructor — atoms hang on send.
2. `centralManager` handling ONE molecule (no `for { }` outer loop) — only the first molecule bonds.
3. Skipping postcommit "done" — daemon races into next molecule before current bond completes.

---

### 3.7 FIFO semaphore

**Scenario.** Counting semaphore where waiters are served in arrival order.

#### Daemon strategy — single goroutine owns count + waiter queue

Composes **S4** + **I12**.

```go
import "container/list"

type FifoSemaphore struct {
    acquireCh chan chan struct{}                            // chan-of-chan — pass reply line
    releaseCh chan struct{}                                 // bare signal
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
                    e.Value.(chan struct{}) <- struct{}{}    // direct hand-off
                } else {
                    count++                                   // bank
                }
            case ch := <-s.acquireCh:
                if count > 0 {
                    count--
                    ch <- struct{}{}                          // immediate wake
                } else {
                    waiters.PushBack(ch)                      // queue
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

**Why no mutex.** `count` and `waiters` are LOCAL variables inside the daemon goroutine. Single-goroutine ownership ⇒ no synchronization primitive needed; the channels do it.

**FIFO comes from Go's per-channel FIFO guarantee.** The acquire channel preserves arrival order; the daemon processes acquires strictly in that order.

**Mistakes worth memorizing.**
1. Two sequential `select`s instead of one with two cases — forces alternation. Use one `select` with multiple cases for "react to whichever fires first."
2. `releaseCh chan chan struct{}` — asymmetry confusion. Acquirers need reply lines (chan-of-chan); releasers don't (bare `chan struct{}`).
3. Forgot `import "container/list"` — cascade of `<X> undefined` errors.

---

### 3.8 Search-Insert-Delete

**Scenario.** Three roles share a list. Searchers concurrent; Inserters at-most-one (compatible with searchers); Deleters exclusive.

#### Implementation — channel-based three-room shape

Composes **I1** + **I2** with the lightswitch idiom encoded as channel-mutex + binary-semaphore.

```go
type SidList struct {
    bibo       chan struct{}                                // cap 1, mutex over searcherCount
    noSearcher chan struct{}                                // cap 1, held while ≥1 searcher in
    noInserter chan struct{}                                // cap 1, held while inserter in
    searcherCount int                                       // PLAIN int — guarded by bibo
    data []int                                              // append-only
    size atomic.Int64
}

func NewSidList(maxSlots int) *SidList {
    return &SidList{
        bibo:       make(chan struct{}, 1),                 // cap 1, available
        noSearcher: make(chan struct{}, 1),
        noInserter: make(chan struct{}, 1),
        data:       make([]int, maxSlots),
    }
}

func (s *SidList) Search(x int) {
    s.bibo <- struct{}{}
    s.searcherCount++
    if s.searcherCount == 1 { s.noSearcher <- struct{}{} }  // first-in claims
    <-s.bibo
    // -- search via atomic-size publication --
    n := s.size.Load()
    for i := int64(0); i < n; i++ { _ = s.data[i] }
    s.bibo <- struct{}{}
    s.searcherCount--
    if s.searcherCount == 0 { <-s.noSearcher }              // last-out yields
    <-s.bibo
}

func (s *SidList) Insert(x int) {
    s.noInserter <- struct{}{}                              // exclude other inserters AND deleters
    n := s.size.Load()
    s.data[n] = x
    s.size.Store(n + 1)                                      // publish
    <-s.noInserter
}

func (s *SidList) Delete() {
    s.noSearcher <- struct{}{}                              // GLOBAL lock order: searcher → inserter
    s.noInserter <- struct{}{}
    // -- delete --
    <-s.noInserter                                           // LIFO release
    <-s.noSearcher
}
```

**Why no counter for inserters.** At most one inserter is ever in — the channel buffer occupancy IS the count. Adding a counter is dead state.

**Mistakes worth memorizing.**
1. Two deleters acquire `noSearcher` and `noInserter` in opposite orders → circular-wait deadlock. Document and enforce a global lock order.
2. Channels primed with `make(chan struct{}, 1)` need to start EMPTY for Pattern B (the lightswitch fits Pattern B). Don't pre-send tokens.
3. Without a turnstile, deleters starve under steady searcher load — same fix as readers-writers (gate-pass for searchers, hold turnstile across deleter's path).

---

## 4. Exam-style scenarios

### 4.1 Channel-based load balancer (2025 Q27)

**Scenario.** A `LoadBalancer` holds N `Server` instances. Each `Server` is responsible for a subset of seats. When a user request comes in, the balancer dispatches it to the right `Server` based on `seatID`. Constraint: **use channels and goroutines only — no `sync` (or other sync primitives).** The user must receive a confirmation of their seat.

**This is fan-out by hash + per-server daemon.** It composes **S6** (channels-only struct) + **S4** (daemon goroutine in constructor) + **I7** (select multiplex).

#### Skeleton with fill-ins (Points A–H)

```go
// Point A — type declarations
type Request struct {
    UserID int
    SeatID int
    Reply  chan bool                                        // each request carries its reply line
}

type Server struct {
    id       int
    requests chan Request
    quit     chan struct{}
    done     chan struct{}                                  // signaled when fully drained
    ts       *TicketServer                                  // provided by the question
}

type LoadBalancer struct {
    servers   []*Server
    numServers int
    maxSeatID int
}

// Constructor for a Server. Runs HandleClient per request in a goroutine.
func NewServer(id, maxSeatID int) *Server {                 // Point C — args
    s := &Server{
        id:       id,
        requests: make(chan Request, 16),                   // some backpressure capacity
        quit:     make(chan struct{}),
        done:     make(chan struct{}),
        ts:       NewTicketServer(maxSeatID),
    }
    go s.run()
    return s
}

func (s *Server) run() {
    defer close(s.done)
    for {
        select {
        case <-s.quit:
            // drain pending requests, then exit
            for {
                select {
                case r := <-s.requests:
                    s.handle(r)
                default:
                    return
                }
            }
        case r := <-s.requests:
            s.handle(r)
        }
    }
}

func (s *Server) handle(r Request) {
    ok := s.ts.HandleClient(r.UserID, r.SeatID)
    r.Reply <- ok
}

func NewLoadBalancer(numServers, maxSeatID int) *LoadBalancer {     // Point B — params
    lb := &LoadBalancer{
        numServers: numServers,
        maxSeatID:  maxSeatID,
    }
    for i := 0; i < numServers; i++ {
        lb.servers = append(lb.servers, NewServer(           // Point C — args
            i, maxSeatID))
    }
    return lb
}

// Dispatch picks the right server by seatID and sends the request.
// Per question: "Server should HandleClient in the background."
func (lb *LoadBalancer) Dispatch(r Request) {                // Point D — signature
    // Point E — body
    seatsPerServer := (lb.maxSeatID + lb.numServers - 1) / lb.numServers
    serverIdx := r.SeatID / seatsPerServer
    if serverIdx >= lb.numServers { serverIdx = lb.numServers - 1 }
    lb.servers[serverIdx].requests <- r                       // background — server's run() handles it
}

func main() {
    rand.Seed(time.Now().UnixNano())
    N := 4

    // Point F — initialize LoadBalancer + collect replies
    lb := NewLoadBalancer(N, 10)
    replies := make([]chan bool, 0, 20)

    for i := 1; i <= 20; i++ {
        seatID := rand.Intn(10)
        // Point G — dispatch
        reply := make(chan bool, 1)                          // buffered = sender doesn't block
        replies = append(replies, reply)
        lb.Dispatch(Request{UserID: i, SeatID: seatID, Reply: reply})
    }

    for i, reply := range replies {                          // Point H — collect & print
        ok := <-reply
        fmt.Printf("user %d: got seat = %v\n", i+1, ok)
    }

    time.Sleep(5 * time.Second)
}
```

**Patterns reused.** S6 (channels-only Server); S4 (daemon goroutine spawned in `NewServer`); I7 (select with quit + work cases); fan-out by `seatID / seatsPerServer`.

**Why no mutex.** Each `Server` is owned by exactly one goroutine reading `s.requests`. That goroutine has exclusive access to `s.ts` — channel-based ownership replaces the mutex. The `LoadBalancer` itself is read-only after construction.

**Why the reply channel is buffered (cap 1).** If `Dispatch` is called from `main` and the reply consumer isn't ready yet, an unbuffered reply would block the server goroutine — head-of-line blocking. Cap 1 lets the server send-and-move-on.

**Common mistakes.**
- Dispatching synchronously (`lb.Dispatch` waits for the reply) — kills the parallelism the question wants. The question says "Server should HandleClient in the background" — fire and collect later.
- Sharing one `replies` channel for all users — replies arrive in dispatch-completion order, not user order. Per-user reply channel makes the answer addressable.
- `rand.Intn(10)` on `seatID` and seat count of 10 — fine here; just match what the question gives.
- Computing `serverIdx := r.SeatID % numServers` — incorrect partitioning if the question says "each Server is responsible for a *subset of seats*" (contiguous range, not striped). Use `r.SeatID / seatsPerServer`.
- Forgetting to allow `numServers > maxSeatID` edge — clamp `serverIdx` or guarantee `numServers ≤ maxSeatID`.

---

### 4.2 Graceful goroutine shutdown (2025 Q28)

**Scenario.** Continuing from §4.1: shut down the `Server` goroutines gracefully once all ticket sales are completed, ensuring no request is left unprocessed. Minimally update §4.1's implementation.

**This is the close-quit + drain-pending pattern.** It composes **I6** (close-broadcast) + **I8** (default for non-blocking drain) + **S5** (graceful shutdown discipline).

#### Diff over §4.1

Add `Stop` methods on `Server` and `LoadBalancer`. The `Server.run` loop already drains pending requests when it sees `<-s.quit` (re-read §4.1's `run` — that's the load-bearing part).

```go
func (s *Server) Stop() {
    close(s.quit)                                            // 1. broadcast: stop accepting new
    <-s.done                                                 // 2. wait for full drain
}

func (lb *LoadBalancer) Stop() {
    for _, s := range lb.servers {
        s.Stop()                                             // serial OR parallel — either is fine
    }
    // (parallel variant: spawn one goroutine per Stop, then WaitGroup-Join all)
}
```

In `main`, after collecting replies:
```go
for i, reply := range replies {
    ok := <-reply
    fmt.Printf("user %d: got seat = %v\n", i+1, ok)
}
lb.Stop()                                                    // *** add this ***
```

**Why `close(s.quit)` and not `s.quit <- struct{}{}`.** Close broadcasts: every blocked `<-s.quit` returns immediately. If you `send`, only one receiver wakes — fine here since each Server has one goroutine, but `close` is the canonical "I'm done" signal.

**Why `<-s.done` after `close(s.quit)`.** `Stop` must be synchronous — caller needs to know "all Server goroutines have fully drained and exited" before main returns. Without `<-s.done`, the test could exit while servers are still mid-handle.

**Why drain inside `run`'s quit case.** Without the inner drain loop, requests buffered in `s.requests` between the last `Dispatch` and `close(s.quit)` are lost. The drain ensures every request that was successfully sent is processed.

**Patterns reused.** I6 (close-broadcast); I8 (default-for-drain); the close→done handshake from S6.

**Common mistakes.**
- `s.quit <- struct{}{}` instead of `close(s.quit)` — works for one server, but breaks if multiple goroutines watch `quit` (and is non-idiomatic).
- Forgetting the inner drain in `run` — buffered requests are dropped on shutdown.
- Forgetting the `done` channel — `Stop` returns before workers actually exit; test races.
- `close(s.requests)` instead of `close(s.quit)` — subsequent `Dispatch` panics with "send on closed channel." Use a separate `quit` channel; `requests` stays open until everyone has stopped sending.

---

### 4.3 Channel-based server with SQ + CQ pipelining (2023 Q8)

**Scenario.** A Go server takes concurrent client connections. Each client sends multiple requests during a session. Submit queue (SQ) and completion queue (CQ) are channels. W workers process requests from SQ, place results in CQ, and send back to the originating client.

**This is the 3-stage pipeline (read → process → write) using channels — fan-out at SQ, fan-in at the per-client reply channel.** Composes **S2** + **I7** (select multiplex) + **I8** (default for non-blocking try) + per-request reply channel from §4.1.

```go
type Request struct {
    queue chan Request      // *** per-client reply channel — fan-in ***
    data  []byte
    // ... other fields ...
}

var SQ, CQ chan Request

func main() {
    SQ = make(chan Request, SIZE)
    CQ = make(chan Request, SIZE)

    // W process workers — fan-out from SQ
    for i := 0; i < W; i++ {
        go func() {
            for {
                select {
                case req := <-SQ:
                    req.process()
                    CQ <- req
                default:
                }
            }
        }()
    }

    // W send workers — read from CQ, route to per-client queue (fan-in by reply chan)
    for i := 0; i < W; i++ {
        go func() {
            for {
                select {
                case req := <-CQ:
                    req.queue <- req                     // route to originating client
                default:
                }
            }
        }()
    }

    // Per-client read + write goroutines (spawned per accept)
    for {
        conn := accept(/* ... */)
        queue := make(chan Request)                       // *** per-client reply channel ***
        // Read goroutine
        go func() {
            for {
                req := conn.read()
                req.queue = queue
                SQ <- req
            }
        }()
        // Write goroutine
        go func() {
            for {
                select {
                case req := <-queue:
                    conn.send(req)
                default:
                }
            }
        }()
    }
}
```

**Concurrency analysis (rubric).**
- **Concurrent tasks:** submission/retrieval to/from SQ and CQ; processing of requests; sending back to clients.
- **Maximum parallelism:** number of worker goroutines.
- **Go patterns used:** for-select (sending and receiving on channels), pipelining (read → process → write), fan-out (SQ → W workers), fan-in (CQ → per-client reply channel).

**Why each request carries its own reply channel.** The CQ is shared by all clients. To route a result back to the *originating* client, each request carries a `queue` field — the per-client write goroutine reads from `queue`. This is the same idiom as G12's chan-of-chan — each request brings its reply line.

**Common mistakes.**
- Single goroutine for both `process` and `send` — turns the pipeline into a serial chain. The point is fan-out at `process`, separate fan-out at `send`.
- One global reply channel for all clients — replies arrive in completion order, not per-client order. Each client needs its own.
- `select { default: }` everywhere — busy-loops the workers. In production you'd remove `default` so the goroutine parks on the empty channel; the rubric shape uses `default` to mean "non-blocking try" but it's unnecessary if SQ and CQ are the only sources.

---

### 4.4 STM (Software Transactional Memory) — Go API (2024 Q11)

**Scenario.** Implement four STM API calls — `stmTxnBegin()`, `stmRd(addr)`, `stmWr(addr, val)`, `stmCommit()` — over a shared `int` memory. Conflict detection at commit. The Go answer uses a **single coordinator goroutine** that owns shared state; transactions communicate via channels.

**This is the daemon-pattern (S4 + I12) at protocol scale.** Single-goroutine ownership of memory means no mutex; channels do the synchronization.

```go
type stmCmd struct {
    op       string                                       // "read", "write", "commit"
    addr     uint32
    value    uint32
    txnID    uint64
    reply    chan stmReply
}

type stmReply struct {
    value      uint32
    committed  bool                                        // for commit replies
}

var coordCh = make(chan stmCmd)

func init() {
    go coordinator()
}

func coordinator() {
    memory := make(map[uint32]uint32)
    txnReadSets  := make(map[uint64]map[uint32]bool)       // per-txn read addrs
    txnWriteSets := make(map[uint64]map[uint32]bool)       // per-txn write addrs
    txnWriteBufs := make(map[uint64]map[uint32]uint32)     // per-txn staged writes

    for cmd := range coordCh {
        switch cmd.op {
        case "read":
            // read-your-own-writes first
            if v, ok := txnWriteBufs[cmd.txnID][cmd.addr]; ok {
                cmd.reply <- stmReply{value: v}
            } else {
                txnReadSets[cmd.txnID][cmd.addr] = true
                cmd.reply <- stmReply{value: memory[cmd.addr]}
            }

        case "write":
            txnWriteSets[cmd.txnID][cmd.addr] = true
            txnWriteBufs[cmd.txnID][cmd.addr] = cmd.value
            cmd.reply <- stmReply{}

        case "commit":
            // Conflict detection: any other in-flight txn that overlaps in writes
            //   with my read OR write set?
            committed := true
            for otherID, otherWrites := range txnWriteSets {
                if otherID == cmd.txnID { continue }
                for addr := range otherWrites {
                    if txnReadSets[cmd.txnID][addr] || txnWriteSets[cmd.txnID][addr] {
                        committed = false
                        break
                    }
                }
                if !committed { break }
            }
            if committed {
                for addr, val := range txnWriteBufs[cmd.txnID] {
                    memory[addr] = val
                }
            }
            // discard txn state
            delete(txnReadSets, cmd.txnID)
            delete(txnWriteSets, cmd.txnID)
            delete(txnWriteBufs, cmd.txnID)
            cmd.reply <- stmReply{committed: committed}
        }
    }
}

// Public API
var nextTxnID uint64 = 0
func stmTxnBegin() uint64 {
    id := atomic.AddUint64(&nextTxnID, 1)
    // initialize empty sets in coordinator (omitted for brevity — could send a "begin" cmd)
    return id
}
func stmRd(txnID uint64, addr uint32) uint32 {
    reply := make(chan stmReply, 1)
    coordCh <- stmCmd{op: "read", txnID: txnID, addr: addr, reply: reply}
    return (<-reply).value
}
func stmWr(txnID uint64, addr uint32, value uint32) {
    reply := make(chan stmReply, 1)
    coordCh <- stmCmd{op: "write", txnID: txnID, addr: addr, value: value, reply: reply}
    <-reply
}
func stmCommit(txnID uint64) bool {
    reply := make(chan stmReply, 1)
    coordCh <- stmCmd{op: "commit", txnID: txnID, reply: reply}
    return (<-reply).committed
}
```

**Why this works without mutexes.** All shared state (`memory`, `txnReadSets`, `txnWriteSets`, `txnWriteBufs`) lives inside `coordinator`'s scope. Only the coordinator reads/writes these maps. Clients communicate via `coordCh` — channels do the synchronization.

**Patterns reused.** S4 (daemon spawned at init); I12 (chan-of-chan / each request carries reply line); §4.1 LoadBalancer's per-request reply pattern.

**Trade-off.** Every operation goes through one goroutine — that goroutine is the bottleneck. For low-conflict workloads, A2's OCC variant (§4.5) avoids it: the coordinator only validates+commits; reads and writes stage locally.

**Common mistakes.**
- Forgetting read-your-own-writes — the coordinator must check `txnWriteBufs[txnID]` before reading `memory`.
- Not cleaning up `txnReadSets`/`txnWriteSets`/`txnWriteBufs` after commit — memory leaks per transaction.
- Buffered `reply` channel of size 0 (rendezvous) — works but means the coordinator blocks until the client receives. Cap-1 lets the coordinator move on if the client is slow.

---

### 4.5 From Assignment 2 — STM via OCC Coordinator (real implementation reference)

**Scenario.** Real CS3211 assignment: Go STM engine, **no `sync` package allowed** (no mutex, no atomics). Reaches "transaction-level concurrency": disjoint transactions commit in parallel; only the validate-and-apply step serializes.

#### Architecture

```
handleConn A:  snapshot → [execute locally]      → commitReq → success → output
handleConn B:  snapshot →   [execute locally]      → commitReq → conflict → retry
                                  ↑ parallel ↑
Coordinator:   snap → snap → ──────────────────── commit → commit  (single goroutine)
```

- **`coordCh chan coordMsg`** — unbuffered; routes both snapshot requests and commit requests to the coordinator.
- **`replyCh chan T`** — buffered(1) per request so the coordinator can send without blocking even if the client has exited.

#### Two-phase OCC

1. **Snapshot phase.** `handleConn` sends `snapshotReq{addrs, replyCh}`. Coordinator reads `memory[addr]` and `versions[addr]` for each address, replies with a consistent `snapshotResp`. Only addresses that READ commands touch (not addresses already written by the same transaction) are requested.

2. **Local execution phase.** `executeLocally` runs purely against the snapshot — **no shared state touched**:
   - **WRITE:** staged in `localWrites`; address added to `writeSetMap`.
   - **READ:** if the address was already written locally → read-your-own-writes (does NOT add to `readSet`); otherwise → use snapshot value and record `(addr, version)` in `readSet`.

3. **Commit phase.** `handleConn` sends `occCommitReq{readSet, writeBuffer, replyCh}`. Coordinator validates the read set against current versions; on conflict the client retries with a fresh snapshot.

#### Why this beats the daemon variant (§4.4)

The §4.4 daemon serializes every read and every write through one goroutine. OCC serializes only the *validate-and-apply* step:
- Two transactions on **disjoint** addresses commit without conflict — full parallelism on local execution.
- Two transactions on **overlapping** addresses: one commits, the other retries with fresh values.

| Concurrency level | This impl |
|---|---|
| 3.3.2 Address-level | ✓ — disjoint transactions commit in parallel |
| 3.3.3 Command-level | ✓ — reads from snapshot, not coordinator memory |
| 3.3.4 Transaction-level | ✓ — OCC; parallel local execution, conflict detected at commit |

#### Coordinator skeleton

```go
type snapshotReq struct {
    addrs   []uint32
    replyCh chan snapshotResp
}
type snapshotResp struct {
    values   map[uint32]uint32
    versions map[uint32]uint64
}
type occCommitReq struct {
    readSet     map[uint32]uint64                          // addr → version-seen
    writeBuffer map[uint32]uint32                          // addr → value-to-write
    replyCh     chan bool
}
type coordMsg struct {                                      // tagged union
    snap   *snapshotReq
    commit *occCommitReq
}

func coordinator(coordCh <-chan coordMsg, ctx context.Context) {
    memory   := make(map[uint32]uint32)
    versions := make(map[uint32]uint64)

    for {
        select {
        case <-ctx.Done(): return
        case msg := <-coordCh:
            switch {
            case msg.snap != nil:
                resp := snapshotResp{
                    values:   make(map[uint32]uint32),
                    versions: make(map[uint32]uint64),
                }
                for _, a := range msg.snap.addrs {
                    resp.values[a]   = memory[a]
                    resp.versions[a] = versions[a]
                }
                msg.snap.replyCh <- resp                    // buffered cap 1 — never blocks
            case msg.commit != nil:
                ok := true
                for addr, seenVer := range msg.commit.readSet {
                    if versions[addr] != seenVer { ok = false; break }
                }
                if ok {
                    for addr, val := range msg.commit.writeBuffer {
                        memory[addr] = val
                        versions[addr]++                    // bump version on commit
                    }
                }
                msg.commit.replyCh <- ok
            }
        }
    }
}
```

#### Client (`handleConn`) skeleton

```go
func handleConn(conn net.Conn, coordCh chan<- coordMsg) {
    txnLog := []stmCmd{/* parsed from conn */}
    for {
        // 1. Snapshot
        addrs := readAddrs(txnLog)                          // addresses READ commands touch
        replyCh := make(chan snapshotResp, 1)
        coordCh <- coordMsg{snap: &snapshotReq{addrs, replyCh}}
        snap := <-replyCh

        // 2. Execute locally (pure, no shared state)
        readSet, writeBuffer := executeLocally(txnLog, snap)

        // 3. Commit
        commitReply := make(chan bool, 1)
        coordCh <- coordMsg{commit: &occCommitReq{readSet, writeBuffer, commitReply}}
        if <-commitReply {
            printOutput(/* ... */)
            return
        }
        // Conflict — retry from step 1
    }
}
```

**Patterns reused.** S4 (single-goroutine coordinator); I7 (select on ctx.Done() + coordCh); reply-channel-per-request; `wg.WaitGroup` (a custom one — A2 forbids `sync.WaitGroup`).

**Common mistakes.**
- Holding the snapshot through the commit RTT — if you need a *consistent* snapshot you must either grab versions and re-validate (OCC) or hold the coordinator (kills concurrency).
- Forgetting to skip read-your-own-writes addresses in the snapshot request — wastes a roundtrip and may give you a stale value compared to your staged write.
- `replyCh` unbuffered — coordinator blocks if the client cancelled mid-transaction. Cap-1 is mandatory for graceful cancellation.
- A2's "no sync" rule applies to `sync.WaitGroup` too — implement your own counting via a channel-based barrier.

---

## 5. Pitfalls catalogue

### Channel semantics

1. **`make(chan struct{})` (cap 0) for binary semaphore.** Cap 0 is RENDEZVOUS, not a semaphore. For mutex semantics you need cap 1.
2. **`nil` channel.** `var ch chan T` (no `make`) blocks forever on send/recv. Force callers through constructors.
3. **Channel buffer ≠ logical occupancy.** Buffered channel holds "queued, not yet picked up." Total in shop = buffer + (1 if cut in progress).
4. **Closing twice or sending on closed.** Runtime panic, not silent breakage. Use a closer goroutine; close once.
5. **`select` without `default:`** is blocking; with `default:` is non-blocking.
6. **Pseudo-random select case selection.** Multiple ready cases → not source-order. Never rely on case order for priority.

### Syntax / grammar

7. **`chan struct {` with newline before `}`.** Parser starts reading a struct type definition. Keep `chan struct{}` on one line.
8. **`} \n else {`.** ASI inserts `;` after `}`, ending the `if`. `} else {` MUST be on one line.
9. **Composite literal field set with `:=`.** Use `:` (key-value), not `:=` (short var decl).
10. **Missing trailing comma** on the last line of a multiline composite literal.
11. **`go func() {...}` without `()`.** `go EXPR` requires a CALL. Always `go func(){...}()`.
12. **Unused locals are compile errors.** Drop the binding (`for range ch`, not `for x := range ch` if `x` is unused).
13. **`struct{}` is the type; `struct{}{}` is the value.**

### `sync.WaitGroup`

14. **Reusing one WG across rounds.** Panic. WG is a count-down latch, NOT cyclic. Use generation-counter cond barrier (I11).
15. **Field as `sync.WaitGroup` value.** `noCopy` lint. Use `*sync.WaitGroup` if you must.
16. **`wg.Add(1)` AFTER `go`.** Race window where Wait may fire before Add registers. Always `wg.Add(1)` BEFORE the `go`.
17. **`wg.Add(N)` inside a struct literal.** Add returns nothing.

### Method receivers / scope

18. **Receiver type mismatch.** `func (s *Shared)` requires the type to be `Shared` (capital). `*shared` (lowercase) is undefined.
19. **Bare field access without receiver inside methods.** Use `c.x`, never `x`. Go has no implicit `this`.

### Worker patterns

20. **`return` from a work arm of a long-running select.** Exits the whole function; worker dies after one job.
21. **`for { select { ... } }` where every arm returns.** Loop never iterates. Drop the `for`.
22. **`case default:`.** No `case` keyword; bare `default:`.

### Counters / accounting

23. **Forgetting `produced.Add(1)` after a send.** Channels synchronize, NOT observe. Counters live OUTSIDE the primitive.

### Signal handling

24. **`make(chan os.Signal)` (unbuffered) for `signal.Notify`.** `Notify` does a non-blocking send; an unbuffered channel with no immediate reader drops the signal. Use `make(chan os.Signal, 1)`.

---

## 6. Build / run / debug

```bash
# Always use -race during development
go run -race .
go test -race ./...
go build -race

# Stress loop — typical "is it really fixed?" test
for i in {1..1000}; do go run -race . || break; done

# Built-in deadlock detector — Go's runtime PANICS automatically with
# "fatal error: all goroutines are asleep - deadlock!"
# This fires when ALL goroutines are blocked. Partial deadlocks (where
# some goroutines still progress) won't trigger it — use SIGQUIT for those.

# pprof for stuck goroutines (PARTIAL deadlock — some goroutines still running)
import _ "net/http/pprof"
go func() { http.ListenAndServe("localhost:6060", nil) }()
# curl localhost:6060/debug/pprof/goroutine?debug=2

# SIGQUIT dumps all goroutine stacks
# Ctrl-\ on a hung program (or `kill -QUIT $PID`)

# goleak for goroutine-leak detection in tests
import "go.uber.org/goleak"
func TestMain(m *testing.M) { goleak.VerifyTestMain(m) }
```

**Bar for "done."** `go run -race` clean + 1000 stress runs + (for tests) `goleak` verifying no leaked goroutines.

**Go's `-race` is best-in-class** among the three race detectors. Always on during development.

---

## 7. Decision matrix — which primitive when

| Need | Reach for | Why |
|---|---|---|
| Bounded buffer | `make(chan T, N)` | Channel **is** a bounded buffer. |
| Mutex | `sync.Mutex`, OR `chan struct{}` cap 1 | Both work; `sync.Mutex` is faster. Channel form is forced when "no sync package" (Q27). |
| Reader-writer | `sync.RWMutex` | Built-in, writer preference. |
| Wait-on-predicate | `sync.Mutex + sync.Cond + for !pred { cv.Wait() }` | No predicate overload; explicit loop. |
| Counting semaphore | `make(chan struct{}, N)` | Pattern B scales naturally. |
| Done / broadcast cancellation | `chan struct{}` + `close()` | One line; substitute for `cv.Broadcast()`. |
| Single-use latch | `sync.WaitGroup` | Counts down once. NOT cyclic. |
| Cyclic barrier | `sync.Mutex + sync.Cond` + generation | `sync.WaitGroup` is single-use. |
| Daemon-orchestrated protocol | goroutine + channels | Single-goroutine ownership ⇒ no mutex needed. |
| FIFO semaphore | daemon goroutine + chan-of-chan | FIFO comes from Go's per-channel FIFO guarantee. |
| Fan-out by key | N per-server channels + dispatcher | §4.1 LoadBalancer shape. |
| Graceful shutdown | `close(quit)` + `<-done` handshake | Quit broadcasts; done makes Stop synchronous. |
| OS signal handling | `signal.Notify` with **buffered** chan | Unbuffered drops signals (non-blocking send). |
| Request-response | `chan ReplyChan` (chan-of-chan) | Reply line per request — addressable response. |

---
