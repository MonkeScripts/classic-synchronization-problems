# Readers-Writers (Go): Mistakes & Learnings

A record of implementing the readers-writers KV-cache in Go — two flavors (`sync.RWMutex` baseline, hand-rolled channel Lightswitch with turnstile) — the bugs along the way, and the single most important cross-language finding: **the same no-starve turnstile algorithm that's flaky in C++ is rock-solid in Go**, because Go's channels guarantee fair wakeup and C++20's `std::counting_semaphore` doesn't.

---

## The headline observation

Three families of takeaway, in ascending order of transferability:

1. **Go-specific syntax and semantics.** The mistakes I hit that were purely "this is how Go works, not how C++ works" — receiver prefixes (`c.x`), trailing commas in composite literals, and above all the channel-buffering semantics that flip the primitive between "handshake" and "semaphore."

2. **Channel-as-semaphore: two patterns, both valid, one more idiomatic.** I arrived at the more Go-idiomatic variant (Pattern B — send = acquire) by instinct. Worth naming explicitly so I can pick it deliberately next time.

3. **The cross-language fairness finding.** The exact same turnstile algorithm I wrote in C++ and Go passed 500/500 stress runs in Go and only ~85% in C++. This isn't an algorithm problem; it's a *runtime guarantee* problem. Go channels promise FIFO wakeup. `std::counting_semaphore` doesn't. The textbook proof assumes fair scheduling, and that assumption is satisfied by one primitive and quietly broken by the other. This is the single most valuable lesson from doing the problem in multiple languages.

---

## The journey at a glance

| # | Mistake | Fix | Category |
|---|---|---|---|
| 1 | `bibo: make(chan struct{})` — unbuffered channel as binary semaphore | `make(chan struct{}, 1)` — capacity 1 | Go primitive semantics |
| 2 | Referring to struct fields without receiver: `bibo <- struct{}{}` (10+ sites) | Prefix every field access with the receiver: `c.bibo <- struct{}{}` | Go syntax |
| 3 | Composite literal fields on separate lines without trailing commas | Every field on its own line needs a trailing comma, even the last | Go syntax |
| 4 | Typo: field declared as `turnstil`, referenced as `turnstile` everywhere else | Fix the declaration, don't rename the uses | Go syntax |
| 5 | Bench harness still only ran `RWMutex` — forgot to `bench("HandRolled", NewKVCacheHandRolled())` in `main()` | Add the second benchmark call | oversight |
| 6 | Stale TODO comments left in the file after the methods were implemented | Cleanup pass — `grep -n TODO main.go` | hygiene |

None of these are algorithm bugs. The algorithm — Lightswitch plus turnstile — transferred directly from the C++ version. The mistakes were all about learning where Go's surface differs from C++.

---

## Deep dive: channels as synchronization primitives

The most transferable concept from this exercise. Once the shape clicks, a lot of Go concurrency code stops looking like magic.

### `chan struct{}` is Go's all-purpose sync primitive

Every use of `chan struct{}` in Go reduces to one of these shapes:

| Shape | Semantics | Use case |
|---|---|---|
| `make(chan struct{})` (unbuffered) | Send blocks until a receive; rendezvous | Handshake between two goroutines. `"I'm done, are you ready?"` |
| `make(chan struct{}, 1)` (cap 1) | Send blocks if full, receive blocks if empty | Binary semaphore / mutex |
| `make(chan struct{}, N)` (cap N) | Limit concurrent senders to N | Counting semaphore / rate limit |
| `close(ch)` on any `chan struct{}` | All receivers get zero value immediately | **Done channel** — broadcast completion |

The last one is worth a section on its own — it's a cornerstone of idiomatic Go. The first three are what we used here.

### Done channels: the family our binary semaphore belongs to

Our `bibo` and `roomEmpty` and `turnstile` are cap-1 channels. They're the binary-semaphore members of the same family as the ubiquitous `done` pattern:

```go
// The canonical Go cancellation pattern
done := make(chan struct{})
go worker(done)
// ...later...
close(done)  // broadcasts to every select/recv in the worker
```

All four shapes in the table above use the **same underlying primitive** — a Go channel — with different capacity, buffering, and whether you `send`, `recv`, or `close`. You don't switch primitives; you switch *how you use one primitive*. That's a very different mental model from C++ where `std::mutex`, `std::condition_variable`, `std::counting_semaphore`, `std::promise`, and `std::future` are five distinct types with different APIs.

In this implementation:
- **`roomEmpty` (cap 1)** — binary semaphore. Only one writer OR the reader collective can hold it.
- **`bibo` (cap 1)** — binary semaphore used as a mutex. Serializes `rc_` manipulation.
- **`turnstile` (cap 1)** — binary semaphore. Writers hold it for their whole critical section; readers gate-pass it.

Three channels, one primitive shape, three different roles via which-operation-when.

### The two binary-semaphore patterns, named

Go's cap-1 channel can encode a binary semaphore two ways — mirror images of each other:

| | Pattern A (textbook translation) | **Pattern B (what I wrote — Go-idiomatic)** |
|---|---|---|
| Initial state | Seeded with one token | Empty |
| Acquire | `<-ch` (take the token) | `ch <- struct{}{}` (put self in; blocks if full) |
| Release | `ch <- struct{}{}` (put back) | `<-ch` (take self out) |
| Mental model | "Here's a token; take it to enter" | "The channel is the set of current holders" |
| Scales to N holders? | No — would need different semantics | **Yes, directly** — just `make(chan struct{}, N)` |

Pattern A is a literal translation of textbook semaphore semantics: acquire = P = take, release = V = put back. It's what C++ intuition leads you to write first.

Pattern B is the Go-native idiom. It's the same pattern you see everywhere else in Go code:
```go
sem := make(chan struct{}, 10)   // up to 10 concurrent
sem <- struct{}{}                // acquire (blocks if 10 already in)
defer func() { <-sem }()         // release
```

The "I put myself in, I take myself out" mental model scales from binary (cap 1) to counting (cap N) without rewriting the code. Pattern A doesn't — you'd have to add a counter or switch primitives.

I stumbled into Pattern B because my instinct said "send when I want to enter, receive when I want to leave." That instinct is Go's preference encoded into my fingers. Worth recognising so I can pick it deliberately next time rather than by accident.

### The crucial detail: buffering matters more than you'd think

```go
make(chan struct{})       // cap 0 — rendezvous, NOT a semaphore
make(chan struct{}, 1)    // cap 1 — binary semaphore
```

The first expression is where my code spent the most time not working. An unbuffered channel isn't just "a tiny buffered channel" — it's a *different synchronization primitive entirely*. A send on an unbuffered channel does not just deposit a value; it blocks until some other goroutine is simultaneously calling receive. `ch <- struct{}{}` with cap 0 means "synchronize with a receiver" — it's only usable if you *have* a receiver to synchronize with.

For a semaphore, you never have a paired receiver waiting — the state (free/held) lives in the channel's buffer. So the moment you forget `, 1`, your first `ch <- struct{}{}` blocks forever.

The Go compiler won't warn you. The race detector won't flag it. The only symptom is: program deadlocks on the first op, goroutines parked on futexes, and you have to go staring at the `make` call.

### `nil` channels vs empty channels — different failure modes

```go
var ch chan struct{}            // nil — send/recv block FOREVER
ch := make(chan struct{}, 1)    // empty buffered — recv blocks; send works (once)
```

A `nil` channel isn't an error. It's a channel that can never be made non-blocking. This bites hardest when you do `new(KVCacheHandRolled)` instead of `NewKVCacheHandRolled()`, because the constructor is where `make` happens — `new` leaves the channel fields at zero-value, which is `nil`. Same shape of bug as unbuffered, same symptom (deadlock on first op).

Best defense: make constructors the only way to build your struct, and if feasible, unexported struct fields that force callers through the constructor.

### Failure-mode table: channels vs C++20 semaphores

| Mistake | C++ `std::counting_semaphore<1>` | Go `chan struct{}` cap 1 |
|---|---|---|
| Missing initialization | Depends on constructor argument | Runtime deadlock forever |
| Over-release (send when full) | **Undefined behavior** — corrupts count silently | **Send blocks forever** — deterministic, findable with `go tool trace` |
| Acquired but never released | Deadlock | Deadlock |
| Used from wrong goroutine | UB | Fine — Go channels have no ownership concept |

The Go failure mode is almost universally better: deterministic hangs beat silent UB every time. The one exception is "used from wrong goroutine" — in C++ that's a bug, in Go it's a feature (lets you transfer ownership between goroutines). Either way, Go's channel model is harder to get *silently* wrong.

---

## Deep dive: the cross-language fairness finding

This is the marquee lesson of the whole readers-writers problem across languages.

### The experiment

Identical turnstile algorithm (Downey slide 12) implemented in C++20 with `std::counting_semaphore<1>` and in Go with `chan struct{}` cap 1. Same benchmark (`NumReaders=8, NumWriters=2, OpsPerThread=1000`).

| Language | Primitive | Stress test result |
|---|---|---|
| C++ | `std::counting_semaphore<1>` | ~17/20 pass, 3/20 timeout (~85% pass rate) |
| Go | `chan struct{}` cap 1 | **500/500 clean under `-race`** |

This is not the algorithm. The algorithm is the same. The primitive is not.

### Why Go wins this one

From the Go Language Specification on channels:

> A channel implements a FIFO queue. Goroutines blocked on a channel are served in FIFO order.

FIFO wakeup is part of the contract. When you `close` or `send` on a channel with N goroutines blocked receiving, the one that's been blocked longest wakes first. No "might be anyone" clause.

### What C++ does (and doesn't) promise

From the C++20 standard, `[thread.sema.cnt]`:

> `release()`: Atomically increments the counter, and if the result is positive, unblocks at least one thread blocked on this semaphore.

"At least one thread." No ordering. The libstdc++ implementation on Linux uses futexes, which aren't strictly FIFO either. In practice it's usually "fair enough"; in a reader-heavy loop on a turnstile, the *first reader blocked on `roomEmpty` while holding `bibo`* sometimes doesn't get woken for long enough that the 3-second test timeout fires. That's a livelock, not a deadlock — some threads keep making progress at the expense of the one thread everything's waiting for.

### The transferable lesson

> **Textbook concurrency pseudocode assumes fair scheduling.** That assumption is usually implicit — the textbook doesn't restate it at every `wait()` call, it just assumes primitives "eventually wake blocked waiters in some reasonable order." Real-world primitives vary in whether they honor that. Before claiming an algorithm is starve-free, check what the underlying primitive actually guarantees. A correctness proof is only as strong as its weakest assumption.

This is the #1 thing I'd want to re-explain to any future self doing concurrent-systems work. The same turnstile "works" or "doesn't work" not based on correctness of the code, but on whether the runtime honors a fairness assumption the algorithm silently depends on.

---

## Deep dive: walking through our implementation

For future reference. Let me annotate the whole thing:

### The state

```go
type KVCacheHandRolled struct {
    data      map[string]string
    counters  Counters            // invariant oracle — never solution state
    bibo      chan struct{}       // cap 1, binary semaphore (mutex)
    emptyRoom chan struct{}       // cap 1, binary semaphore
    turnstile chan struct{}       // cap 1, binary semaphore
    rc        int                 // reader count, guarded by bibo
}
```

- `bibo` protects `rc`. All reads and writes of `rc` happen between `c.bibo <- struct{}{}` and `<-c.bibo`. Because of that, `rc` can be a plain `int` — no `atomic.Int32` needed.
- `emptyRoom` is held while any reader or writer is in the "room." Acquired by the first reader, released by the last reader. Acquired by any writer, released by that same writer.
- `turnstile` is the no-starve gate. Writers hold it from the start of their critical section to the end. Readers gate-pass it (acquire, release immediately).
- `counters` is the invariant oracle — an independent witness. Separate from `rc` by design (see C++ learnings doc for the full argument).

### Reader (Get) — step by step

```go
func (c *KVCacheHandRolled) Get(k string) string {
    c.turnstile <- struct{}{}             // 1. Gate-pass: acquire turnstile.
    <-c.turnstile                         // 2. Release immediately.
    c.bibo <- struct{}{}                  // 3. Acquire bibo (mutex for rc).
    c.rc++                                // 4. Increment reader count.
    if c.rc == 1 {                        // 5. First reader?
        c.emptyRoom <- struct{}{}         //    Yes — acquire roomEmpty.
    }
    <-c.bibo                              // 6. Release bibo.
    c.counters.EnterRead()                // 7. Invariant: assert we're legit.
    v := c.data[k]                        // 8. THE ACTUAL READ.
    c.counters.ExitRead()                 // 9. Invariant update.
    c.bibo <- struct{}{}                  // 10. Acquire bibo again.
    c.rc--                                // 11. Decrement.
    if c.rc == 0 {                        // 12. Last reader?
        <-c.emptyRoom                     //     Yes — release roomEmpty.
    }
    <-c.bibo                              // 13. Release bibo.
    return v
}
```

Key design properties:

- **Lines 1–2 are the whole no-starve mechanism for readers.** A reader that arrives while a writer holds the turnstile blocks at line 1 until the writer is done. A reader that arrives when no writer is queued blazes through both lines instantly.
- **Line 5's if is inside the bibo critical section.** If it weren't, two simultaneous "first" readers could both try to acquire `emptyRoom`, over-sending it — which in Go would just deadlock the second sender, which is findable but annoying.
- **Line 7 (`EnterRead`) is outside bibo.** By the time we reach it, we know `emptyRoom` is held (either by us or by a prior first reader). That means no writer can currently be in. So the invariant check passes. Calling it outside bibo means shorter critical section and more reader-reader concurrency through the actual map read.
- **Line 12's if is also inside the bibo critical section.** Otherwise two "last" readers could both observe `rc == 0` and both try to release `emptyRoom`, over-sending it.

### Writer (Set) — step by step

```go
func (c *KVCacheHandRolled) Set(k, v string) {
    c.turnstile <- struct{}{}             // 1. Acquire turnstile.
    c.emptyRoom <- struct{}{}             // 2. Acquire roomEmpty — may block.
    c.counters.EnterWrite()               // 3. Invariant check.
    c.data[k] = v                         // 4. THE WRITE.
    c.counters.ExitWrite()                // 5. Invariant update.
    <-c.emptyRoom                         // 6. Release roomEmpty.
    <-c.turnstile                         // 7. Release turnstile.
}
```

Key design properties:

- **Lines 1 and 7 fence the entire critical section.** While a writer is between these two lines, no new reader or writer can even *start* their entry path — they block at the turnstile.
- **Line 2 may block for a long time** if there are readers currently in the room. That's fine — this is the writer waiting for the reader collective to drain. While it waits, the turnstile is held (line 1), so no new readers can join the collective. Existing readers will eventually finish, the last one hits line 12 of `Get`, releases `emptyRoom`, and we unblock.
- **Release order in lines 6-7 is immaterial for correctness.** Either order works. I went with reverse acquisition order (LIFO), which is the habit from C++ RAII and lock-ordering convention.

---

## Final results

| Implementation | 500-run stress w/ `-race` | Mean time (8R/2W/1000 ops) |
|---|---|---|
| `KVCacheRW` (sync.RWMutex) | 500/500 ✓ | ~18 ms |
| `KVCacheHandRolled` (Lightswitch + turnstile) | **500/500 ✓** | ~24 ms |

The `RWMutex` baseline beats the hand-rolled version on this workload by ~33%, which is the expected Go story: `sync.RWMutex` is heavily optimized and the channel ops have more overhead per call. The comparison gets *interesting* at higher contention or larger critical sections — worth re-running at `50R/1W` with a microsecond sleep in the read CS to see if the Lightswitch closes the gap or the `RWMutex` pulls further ahead.

---

## Working code (full class, for reference)

```go
type KVCacheHandRolled struct {
    data      map[string]string
    counters  Counters
    bibo      chan struct{}
    emptyRoom chan struct{}
    turnstile chan struct{}
    rc        int
}

func NewKVCacheHandRolled() *KVCacheHandRolled {
    return &KVCacheHandRolled{
        data:      make(map[string]string),
        bibo:      make(chan struct{}, 1),
        emptyRoom: make(chan struct{}, 1),
        turnstile: make(chan struct{}, 1),
    }
}

func (c *KVCacheHandRolled) Get(k string) string {
    c.turnstile <- struct{}{}
    <-c.turnstile
    c.bibo <- struct{}{}
    c.rc++
    if c.rc == 1 {
        c.emptyRoom <- struct{}{}
    }
    <-c.bibo
    c.counters.EnterRead()
    v := c.data[k]
    c.counters.ExitRead()
    c.bibo <- struct{}{}
    c.rc--
    if c.rc == 0 {
        <-c.emptyRoom
    }
    <-c.bibo
    return v
}

func (c *KVCacheHandRolled) Set(k, v string) {
    c.turnstile <- struct{}{}
    c.emptyRoom <- struct{}{}
    c.counters.EnterWrite()
    c.data[k] = v
    c.counters.ExitWrite()
    <-c.emptyRoom
    <-c.turnstile
}
```

Three channels, one `int`, and a map. The shape mirrors the C++ version almost line-for-line, with channel ops replacing semaphore acquire/release.

---

## Cross-language summary

If I had to distill the reader-writer problem across all three languages into one paragraph:

> The algorithm is the same everywhere — Lightswitch for the base case, turnstile for the no-starve variant. The *primitive* changes. In C++ I used `std::counting_semaphore<1>`, which has correct mutual-exclusion but unspecified wakeup fairness, and the turnstile variant was flaky. In Go I used `chan struct{}` cap 1, which the spec says wakes goroutines in FIFO order, and the exact same algorithm ran 500/500 clean. The code looks almost identical; the runtime guarantees it depends on don't. That gap — between what the pseudocode assumes and what the language gives you — is the whole intellectual content of doing this exercise in multiple languages.

---

## Next moves

- [ ] **Add `KVCacheMu` (plain `sync.Mutex`).** Three-line class, data point for the "at what R:W ratio does RWMutex beat Mutex?" question the template's header comment is asking.
- [ ] **Writer starvation experiment.** Current constants (`8R/2W, 1000 ops`) don't actually exercise the Lightswitch's starvation property. Bump to `50R/1W, 10000 ops`, add `time.Sleep(time.Microsecond)` inside `Get`'s critical section, and measure per-writer max wait time by sampling `time.Now()` around the `c.emptyRoom <- struct{}{}` in `Set`. Expected: Lightswitch-without-turnstile shows growing max wait, Lightswitch-with-turnstile holds steady, `RWMutex` also holds steady (since Go's RWMutex has writer preference since Go 1.x).
- [ ] **Sabotage experiments to cement channel semantics:**
  - Change one `make(chan struct{}, 1)` to `make(chan struct{})` → predict exactly what deadlocks and why.
  - Forget one initialisation in the constructor (leave a channel as `nil`) → predict the failure mode.
  - Move a `c.rc++` outside the `bibo` critical section → predict what the `-race` detector says.
- [ ] **Connect the cross-language finding to existing learnings docs.** The C++ readers-writers-learnings.md already flags the flakiness; this Go doc should be referenced from there as the confirmation that the diagnosis was correct. (And the PC/Rust docs already have parallel points about primitive semantics — cross-references would help a future reader see the pattern across problems.)
