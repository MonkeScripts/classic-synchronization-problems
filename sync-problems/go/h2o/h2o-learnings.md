# H2O (Go): Mistakes & Learnings

A record of implementing the water-factory problem in Go, covering both the **daemon-goroutine** strategy (in `go/h2o/`) and the **leader-election** strategy (in `go/h2o_leader/`), the bugs along the way, and the conceptual model that made the designs click.

---

## The headline observation

The daemon and leader strategies have the **same protocol shape** — precommit, commit, bond, postcommit — but different **ownership of the loop**:

- **Daemon:** a separate goroutine runs `for { ... }`. Atoms submit and wait. Simple, but the goroutine leaks if you don't wire up shutdown.
- **Leader:** each oxygen *is* one iteration of the daemon's loop. A `oxygenMutex` (channel of capacity 1) ensures only one runs at a time. No long-running goroutine, no leak.

The hydrogen code is *identical* in both — hydrogens don't know or care whether the responder is a daemon or a peer oxygen acting as ad-hoc daemon.

Once you see it, the daemon design is a **client/server pattern** in disguise:

- **Atoms = clients.** Show up independently, submit a request, wait for response, do the work, signal done.
- **Daemon (or leader oxygen) = server.** Single loop: collect 2H + 1O requests, dispatch a "go bond" signal to those three, wait for all three to finish, repeat.
- **`commit` channel = private reply channel.** A back-channel addressed to one specific atom. The shared `RequestH` / `RequestO` are the *public mailboxes*; sending a `commit` channel through one of them is how an atom gives the server its personal phone line.

This is a common Go idiom: when you need a response addressed to a *particular* sender (not "anyone listening"), you send a channel through a channel.

---

## Why this design exists (the per-atom requirement)

A simpler "buffered channel as semaphore" approach is tempting:

```go
hChan := make(chan struct{}, 2)
oChan := make(chan struct{}, 1)
```

…but it doesn't satisfy the actual problem. The H2O constraint is:

> Each hydrogen/oxygen is its own goroutine, and **each must call its own bond()**. The three bond() calls forming one molecule must all execute together — no atom may bond until 2H + 1O have arrived, and those exact three atoms bond before any 4th proceeds.

If atoms just sent on `hChan`/`oChan` and then independently called `bond()`, there's no coordination — five H atoms could call `bond()` interleaved, never properly grouped.

If a central goroutine drained the channels and called `bond()` itself, the original atom goroutines have already moved on. They never bond *as themselves*.

The daemon/leader designs specifically give the green light back to *those three atom goroutines*. The commit-channel-through-channel trick is what makes that addressable.

---

## The two-turn conversation on `commit`

The same channel sees four operations, perfectly paired:

```
Server (daemon or leader)        Atom (e.g. H1)
-------------------------        --------------
                                 commit := make(chan struct{})
<-RequestH            ←──────    RequestH <- commit            (precommit)

h1 <- struct{}{}      ──────→    <-commit                      (commit / "go")
                                 bondH()
<-h1                  ←──────    commit <- struct{}{}          (postcommit / "done")
```

`struct{}{}` carries no data — the **act of sending is the message**. Pure synchronization.

### Why the postcommit `commit <- struct{}{}` is needed (the subtle part)

It's tempting to drop this — "the response from the server is already done after `<-commit`, why send back?" The answer: it's a **completion barrier**.

Without it, the server would loop back and admit the next molecule's atoms while the current molecule is still mid-`bond()`. Molecule #2's H atoms could be released before molecule #1's atoms finished bonding — violating "atoms bond as a synchronized group of 3".

The server's trailing `<-h1; <-h2; <-o` after sending "go" is what stops it from racing ahead. Same channel, opposite direction, second phase.

---

## Daemon strategy: mistakes I hit

### 1. `:=` inside a struct literal

```go
return &WaterFactory{
    RequestH := make(chan chan struct{})   // ❌
}
```

Struct field initializers use `:` (key-value), not `:=` (short variable declaration). Inside a composite literal you're naming *fields*, not declaring variables. Easy slip when you've been writing a lot of `x := ...` elsewhere.

```go
return &WaterFactory{
    RequestH: make(chan chan struct{}),    // ✅
}
```

### 2. Missing commas in struct literal

```go
return &WaterFactory{
    RequestH: make(chan chan struct{})     // ❌ no comma
    RequestO: make(chan chan struct{})
}
```

Go composite literals require a trailing comma after *every* element when each is on its own line — including the last. Compiler error: `unexpected newline in composite literal`.

### 3. Forgot to spawn the daemon goroutine

```go
func NewWaterFactory() *WaterFactory {
    return &WaterFactory{
        RequestH: make(chan chan struct{}),
        RequestO: make(chan chan struct{}),
    }
    // ❌ never `go centralManager(wf)`
}
```

The factory had the channels but no daemon to read from them. Atoms blocked forever on `wf.RequestH <- commit` because no goroutine was on the other end. The build was clean — only running it surfaced the deadlock.

```go
func NewWaterFactory() *WaterFactory {
    wf := &WaterFactory{
        RequestH: make(chan chan struct{}),
        RequestO: make(chan chan struct{}),
    }
    go centralManager(wf)   // ✅
    return wf
}
```

### 4. `centralManager` only handled ONE molecule

```go
func centralManager(wf *WaterFactory) {
    h1 := <-wf.RequestH
    h2 := <-wf.RequestH
    o  := <-wf.RequestO
    // ... full protocol ...
    // ❌ function returns — only one molecule ever forms
}
```

The first molecule completed; the daemon goroutine exited; every subsequent atom hung. Fix: wrap in `for { }`.

The lecture's "Aside: Downsides of a daemon based approach" calls out the *opposite* mistake — the daemon's loop never exits, so the factory never gets GC'd. Both have the same root: **the daemon's lifetime is an explicit design choice, not something the language hands you.**

---

## Leader strategy: mistakes I hit

### 5. Channel-as-mutex without the buffer

The leader needs a non-reentrant mutex. There are two valid Go-mutex idioms:

**Lecture pattern: receive-to-take, send-to-release, pre-filled.**
```go
mu := make(chan struct{}, 1)
mu <- struct{}{}           // pre-fill: "lock is available"

<-mu                       // take (drains the slot)
mu <- struct{}{}           // release (refills the slot)
```

**Inverted pattern: send-to-take, receive-to-release, no pre-fill.**
```go
mu := make(chan struct{}, 1)
// no pre-fill — empty slot = "available"

mu <- struct{}{}           // take (fills the slot)
<-mu                       // release (drains the slot)
```

Both work. The buffer state — empty vs full — is the lock-held flag either way.

**My bug:** used the inverted idiom but forgot the buffer:
```go
OxySem: make(chan struct{}),     // ❌ capacity 0
```

With capacity 0, `OxySem <- struct{}{}` is an unbuffered send — blocks until *another goroutine* receives. But the only receiver is at the *end of the same function*, in the same goroutine. **A single goroutine can't satisfy its own send by reaching a later receive** — it executes one statement at a time, so if it's blocked on the send, it can never advance to the receive that would unblock it. Self-deadlock on the very first oxygen.

### Fix
```go
OxySem: make(chan struct{}, 1),   // ✅
```

With cap 1, the send goes into the buffer slot and the goroutine moves on. The buffer holds the "lock-held" token until the same goroutine drains it at the end. No second goroutine required.

### Mental model

> **Unbuffered channel = rendezvous between two goroutines.** Send blocks until another goroutine receives. Use this when there's always a peer on the other side.
>
> **Buffered cap-N channel = a tiny queue.** You can fill from one goroutine and drain from another (or the same) later. Mutex-as-channel needs queue semantics, hence cap 1.

In `oxygen()`, both the take *and* the release happen in the same goroutine — so we need queue semantics → cap 1.

Compare to `commit` in `hydrogen()`: that one IS unbuffered and works fine because hydrogen and the leader oxygen are *different* goroutines. Two-goroutine rendezvous, no buffer needed.

### 6. Bond order: signal partners BEFORE bonding

```go
wf.OxySem <- struct{}{}
h1 := <-wf.RequestH
h2 := <-wf.RequestH
bondO(id)               // ❌ leader bonds alone first
h1 <- struct{}{}        // then signals H atoms
h2 <- struct{}{}
```

In this order, the leader bonds *solo* (oInBond=1), finishes, and only then signals h1 and h2 to start. The harness's `max_total_in_bond` invariant would never reach 3 — bonds run serially, not as a synchronized group.

```go
h1 <- struct{}{}        // ✅ signal first
h2 <- struct{}{}
bondO(id)               // bond concurrently with h1 and h2
<-h1
<-h2
```

The whole point of the leader scheme is that all 3 atoms bond *at the same time*. Signaling has to come before the leader's own `bondO`, so the three calls overlap.

---

## Summary: the Go muscle memory I want to keep

### Syntax / language
1. **`:` not `:=` inside struct literals.** Field initialization is not variable declaration.
2. **Trailing commas on every line of a composite literal.** Including the last.

### Channel idioms
3. **`chan chan struct{}` is the public mailbox; the inner `chan struct{}` is the private reply line.** Send a channel through a channel when you need a response addressed to a specific sender.
4. **The same channel can carry a two-turn conversation** (go-then-done) as long as the turns are temporally separated. No race, no ambiguity.
5. **Unbuffered channel = rendezvous between two goroutines. Buffered cap-N = a tiny queue.** Same-goroutine take-and-release needs queue semantics → cap 1; two-goroutine rendezvous → unbuffered works.
6. **Channel-as-mutex requires `make(chan struct{}, 1)`**, not unbuffered. Pick a direction (take vs release) and stick with it; either works as long as the channel is buffered.

### Protocol
7. **A type containing channels is not a service — until something `go`-spawns a goroutine to read them.** The factory does no work unless you start the daemon.
8. **A long-running daemon's loop body is the *protocol per round*, not the whole program.** Wrap in `for {}` once you're sure the body is right.
9. **Postcommit "done" is a completion barrier**, not just politeness. Without it, the server races ahead into the next molecule while the current one is still bonding.
10. **Leader signals partners *before* doing its own work.** Otherwise "bond as a synchronized group" degenerates into serial bonding.

### Strategy choice
11. **Daemon vs leader trade-off:** daemon is simpler per-atom but leaks a goroutine; leader has slightly more code per atom but no long-running goroutine. For finite test runs daemon is fine; for long-lived services leader scales better.
