# Dining Philosophers (Go): Mistakes & Learnings

A record of implementing dining philosophers in Go across two strategies — channel-per-chopstick with the **odd/even ring** (asymmetric ordering, lecture slide 31), and a **try-and-back-off** variant (the channel equivalent of C++'s `std::scoped_lock`). The algorithm choice was right both times; almost every bug was Go-syntax slippage. This doc skews toward fundamentals (`make`, `struct{}{}`, method receivers, semicolon insertion, labeled break, non-blocking select) because that's where the gaps were.

---

## The headline observation

Three families of takeaways:

1. **Algorithm right, implementation wrong.** The odd/even ring is a clean break of the lock-acquire-graph cycle: even-pid grabs left first, odd-pid grabs right first, no thread holds one chopstick while waiting on the next-in-cycle. That part I got first try. The compile errors were all about Go's surface — type vs value, array vs slice, where Go inserts semicolons, what `chan struct{}` means as a return type.

2. **Go syntax has fewer rules than C++ but enforces them harder.** Brace placement isn't style — it's grammar. Forgetting one `}` after `chan struct {` doesn't give a "missing brace" error; it gives "unexpected keyword if" because the parser is now inside a *struct type definition* expecting fields. Reading Go errors literally is a skill of its own.

3. **Cross-language comparison: race detector vs TSan.** The C++ footman tripped TSan's lock-order-inversion because the chopstick acquire graph still had a cycle (the cap was a *runtime* prevention, invisible to static analysis). The Go asymmetric strategy here passes `-race` clean because the lock-acquire graph **genuinely has no cycle** — P1 and P3 reverse direction, breaking it structurally. Same problem, different prevention mechanism, different analyzer outcome.

---

## The journey at a glance

| # | Mistake | Fix | Category |
|---|---|---|---|
| 1 | `func (s *Shared) getOddIdxChopstickCh(pid int) chan struct {` (newline before `}`) | `chan struct{} {` on one line | Go grammar |
| 2 | `} \n else { ... }` | `} else {` on same line | Go semicolon insertion |
| 3 | `func (s *shared) Init()` — type is `Shared` (capital S) | `*Shared` | typo, but shape of method receivers |
| 4 | `s.chopstickChs = make([]chan struct{}, 0, N)` — making a slice into an array field | Delete the line; arrays are zero-init by Go | arrays vs slices |
| 5 | `evenIdxCh <- struct{}` | `evenIdxCh <- struct{}{}` | type vs value |
| 6 | `evenIdxCh <- struct{}{}` then `evenIdxCh <- struct{}{}` again (typo: should be `oddIdxCh`) | Send to the matching channel | algorithm |
| 7 | `Init()` defined but never called from `main()` | `s.Init()` after `s := &Shared{}` | linkage |
| 8 | `rigthCh <- struct{}{}` (typo for `rightCh` in try-backoff Phase B) | `rightCh` | typo |
| 9 | Try-backoff `eatTryBackoff` had no release at function end — held both chopsticks forever after first meal | Add `leftCh <- struct{}{}` and `rightCh <- struct{}{}` after the eating section | algorithm |

Compare to the C++ writeup: those mistakes were "is the resource a single mutex or per-chopstick? are the locks shared or per-call?" Here they're "do I have a value or a type? a slice or an array? same line or different?" Different friction, same trial-and-error rhythm.

---

## Deep dive: Go syntax fundamentals (the things I didn't actually know)

The user asked for this section explicitly. Each subsection covers one thing I tripped on plus the wider rule it's an instance of.

### 1. Arrays vs slices vs `make`

In Go these are **two different types** and you have to know which you have. They look similar:

```go
var a  [5]int          // ARRAY — fixed size, baked into the type
var b  []int           // SLICE — variable size, three-word header pointing into a backing array
```

Key differences:

| | Array `[N]T` | Slice `[]T` |
|---|---|---|
| Size | Part of the type — `[5]int` and `[6]int` are different types | Not part of the type |
| Storage | The N elements live inline in whatever holds the array (struct field, stack, etc.) | Three-word header `(ptr, len, cap)` pointing to a separately-allocated backing array |
| Pass to function | **By value** (copies all N elements) — passing big arrays is expensive | By value, but the value is just the header — backing array is shared |
| Created with | Zero-initialized automatically by Go (`var a [5]int` has a usable, all-zero array immediately) | `make([]T, len, cap)` (header + backing array) or `[]T{...}` literal |
| `make` works on it? | **No.** `make` is for slices, maps, channels. | Yes. |

My bug:

```go
chopstickChs [N]chan struct{}                         // field is an ARRAY
...
s.chopstickChs = make([]chan struct{}, 0, N)          // RHS is a SLICE
```

Three things wrong at once:
- Type mismatch — can't assign a `[]chan struct{}` (slice) to a `[N]chan struct{}` (array).
- Even if the types matched, `make([]T, 0, N)` makes a slice with **length 0**, capacity N — there's nothing in it. The "0" means "the slice starts empty"; you'd need `len = N` to get N usable entries.
- And even if length were N, those entries would all be zero-valued (`nil` for channels). For chopsticks-as-channels you still need to `make()` *each individual channel* — `make` on the outer doesn't recursively initialize the inner channel objects.

The right shape for this exercise:

```go
chopstickChs [N]chan struct{}     // array — already exists, zero-initialized to nil channels

func (s *Shared) Init() {
    for i := range s.chopstickChs {                   // range over the existing array
        s.chopstickChs[i] = make(chan struct{}, 1)    // make EACH channel (cap 1)
        s.chopstickChs[i] <- struct{}{}               // prime with one token
    }
}
```

No `make` on the array — arrays don't need it. `make` only on each `chan struct{}`.

### 2. `struct{}` the type vs `struct{}{}` the value

This one bit me on the channel send:

```go
evenIdxCh <- struct{}      // type used as a value — error
evenIdxCh <- struct{}{}    // empty struct VALUE
```

Decompose `struct{}{}`:

- `struct{}` — the *empty struct type*. A struct with zero fields.
- `T{ ... }` — a struct literal of type `T`, with fields filled in. With zero fields, the literal is just `T{}`.
- So `struct{}{}` parses as: `struct{}` (the type) + `{}` (the literal of that type with no fields). Awkward to read but logically obvious once you decompose it.

Why use `struct{}` at all?

- A `chan T` carries a `T` per send. Make `T` as small as possible.
- `chan bool` works but uses 1 byte per item; the boolean value is wasted because you only ever send `true`.
- `chan struct{}` uses **0 bytes** per item — Go optimizes the empty struct to take no space. The signal is purely "a send happened" / "a receive happened"; there is no payload at all.

So `chan struct{}` is the canonical Go "no-data signal channel" — used for completion notifications, semaphore tokens, broadcast cancellation (`close(done)`), barrier release, etc. You'll see this everywhere in idiomatic Go.

The same `struct{}{}` value-construction pattern works for any type:

```go
type Point struct { X, Y int }
p := Point{X: 3, Y: 4}     // struct literal with named fields
q := Point{3, 4}           // struct literal with positional fields
e := struct{}{}            // empty struct literal — no fields to specify
```

### 3. Method receivers — what `func (s *Shared) Foo()` actually means

I typed `*shared` (lowercase) when the type was `Shared`. The fix was trivial; the *concept* is worth a paragraph because Go's method system is different from Java/C++ in load-bearing ways.

```go
func (s *Shared) Init() {
    s.chopstickChs[0] = make(chan struct{}, 1)
}
```

Anatomy of the receiver:

- `(s *Shared)` — the receiver clause. Reads as "this method is on `*Shared` (a pointer to a `Shared`), and inside the method body the receiver is named `s`."
- `s` is just a local name, like a function parameter. It can be anything — `(self *Shared)`, `(this *Shared)`. Convention is a short letter, not `this` or `self`.
- `*Shared` is the receiver type. This is **not** an inheritance/method-table thing the way Java has it. In Go, methods are namespaced under their receiver type, but the type and its methods are *separate* declarations.

Pointer vs value receiver — this matters:

```go
func (s *Shared) Init()           // pointer receiver — modifications stick
func (s Shared)  Init()           // value receiver — `s` is a copy; modifications are lost
```

Use pointer receivers when:
- The method modifies the receiver (mutating state).
- The struct is large and copying would be expensive.
- Consistency: if any method on the type uses pointer receivers, all of them should, to avoid the receiver-method-set surprises (a story for another day).

Use value receivers for small, immutable-ish types (`time.Time`, small wrapper types).

In our case: `Init()`, `checkInvariant()`, `getLeftChopstickCh()`, etc. — all should be pointer receivers, because the channels and atomics inside `Shared` are stateful and we never want to copy the struct mid-game.

What about call-site syntax?

```go
s := &Shared{}     // s is *Shared
s.Init()           // calls the pointer-receiver method directly

s2 := Shared{}     // s2 is Shared (value)
s2.Init()          // also works — Go auto-takes the address: equivalent to (&s2).Init()
```

Go silently inserts `&` or `*` to match. So you usually don't think about it at the call site — but you do at the *declaration* site, because the method's receiver type determines what's mutable.

The mistake I made (`*shared`) was just a typo. But understanding why `s` and `Shared` and `*Shared` are three separate things — the receiver name, the type, and the type's pointer — is what made the typo even possible. They're easy to conflate when you're new to Go.

### 4. Implicit semicolons and why brace placement is grammar

I wrote:

```go
} 
else {
    ...
}
```

…and got a syntax error. The reason is **Go's automatic semicolon insertion (ASI)**. The Go grammar uses semicolons as statement terminators, but the *language spec* says: at end of every line, if the last token is one of a specific set (an identifier, a literal, certain keywords like `return`/`break`/`continue`/`fallthrough`, or a closing `)`/`]`/`}`), the lexer automatically inserts a `;`.

`}` is in that set. So after my `}`, ASI inserts a `;`. The parser sees:

```go
} ;       // end of the if-block; semicolon terminates the if-statement.
else {    // there's now no preceding `if` to attach this `else` to → syntax error
```

Same rule explains:

- `} else {` *must* be on one line (otherwise ASI ends the `if`).
- `} else if cond {` must be on one line.
- Function opening `{` must be on the same line as the function header (`func foo() {` not `func foo()` then `{`) — same reason; otherwise ASI ends the declaration.

This is the rule that gives Go's brace style its uniformity. There is no "do I put `{` on the next line?" debate — you can't, the language won't let you.

### 5. The `chan struct {` line-break trap

Probably the most confusing error message I got:

```go
func (s *Shared) getOddIdxChopstickCh(pid int) chan struct {     // ← intended: chan struct{}
    if pid % 2 == 1 {
        return s.getLeftChopstickCh(pid)
    }
    ...
}
```

Compiler error: `unexpected keyword if, expected field name or embedded type`.

Why? The parser sees `chan struct {` and starts reading a *struct type definition* whose body is everything until the matching `}`. Inside a struct definition it expects field declarations like `Name Type` — and `if` is not a valid field name. So the error is *correct* but only makes sense once you realize the parser thinks it's inside a struct body, not a function body.

The intended type was `chan struct{}` — "channel carrying empty-struct values." The whole `struct{}` needs to fit on the same line so the parser closes the type before hitting the function-body brace.

Two ways to write it:

```go
func foo() chan struct{} { ... }                           // single line, what I want
func foo() chan struct{}                                   // declaration-only; no body
{ ... }                                                    // wouldn't compile anyway (ASI)
```

And the "alias if you want it readable" approach:

```go
type Token = struct{}        // alias the empty struct type
type Sem   = chan Token      // alias the channel type
func foo() Sem { ... }       // now the line stays short
```

Aliases are nice when the type is reused; for a one-off, just keep it on one line.

### 6. Labeled break and non-blocking select (added with the try-backoff variant)

Two Go idioms I'd never used before, and both are central to expressing try-and-back-off cleanly.

**Labeled break.** A bare `break` inside a `select` only escapes the current case, not the surrounding `for`. To exit a for-loop from inside a nested select (or switch), you need a label:

```go
acquire:
for {
    <-leftCh
    select {
    case <-rightCh:
        break acquire    // exits the for-loop, NOT just the select
    default:
        // fall through, retry
    }
    leftCh <- struct{}{} // give it back
}
```

The label `acquire:` sits on the for-loop. `break acquire` reads as "break out of the labeled construct named `acquire`." `continue` works the same way (`continue acquire` skips to the next iteration of that loop). This is the canonical Go answer to "how do I escape multiple levels of control flow without a flag variable?" — every other mainstream language has either a `goto` you avoid or a flag-based pattern; Go has labeled break/continue baked into the grammar specifically because nested selects are common.

You'll typically see labels in three places: nested loops, loops containing selects, and rarely, switches inside loops.

**Non-blocking select.** A `select` with no `default:` blocks until one of its cases is ready (Go's standard "wait on whichever channel fires first" idiom). Add a `default:` case and the semantics flip: `select` tries each case once, and if none are immediately ready, runs the `default:` and falls through.

```go
// blocking — wait until rightCh has a value
select {
case <-rightCh:
    // got it
}

// non-blocking try
select {
case <-rightCh:
    // got it RIGHT NOW
default:
    // not ready, keep going (don't wait)
}
```

Both patterns use the same keyword. The presence or absence of `default:` is what flips between "wait" and "try." This is also how you write timeout patterns — the `default:` slot is replaced by `case <-time.After(d):` to mean "wait, but give up after `d`."

For try-and-back-off, the non-blocking try is what makes it possible to attempt a second channel without committing to wait for it. If you couldn't get the second one immediately, you fall through to `default:`, give back the first, and try the other order.

---

## Deep dive: the second strategy — try-and-back-off

The Go equivalent of C++'s `std::scoped_lock{a, b}` algorithm. Block-receive on one chopstick, then non-blocking-try the other; if the try fails, release the first and retry the *other* order. Either both are held at the moment of `break acquire`, or neither is held at the bottom of the loop body — never one held while another philosopher waits on it as their first.

```go
acquire:
for {
    // Phase A: try left-first
    <-leftCh
    select {
    case <-rightCh:
        break acquire        // both held — done
    default:
    }
    leftCh <- struct{}{}     // didn't get right — give back left

    // Phase B: try right-first (symmetric)
    <-rightCh
    select {
    case <-leftCh:
        break acquire
    default:
    }
    rightCh <- struct{}{}    // give back right, loop iterates
}
... eat ...
leftCh <- struct{}{}         // release at function end
rightCh <- struct{}{}
```

### Why no cycle in the lock-acquire graph

This is the subtle thing. In the odd/even ring, the cycle was prevented *structurally* — odd philosophers literally never grab their left-then-right, so the graph has no cycle. In try-and-back-off, every philosopher has the *same* code (no parity asymmetry), and Phase A still goes left-then-right. So at first glance: cycle?

Answer: it's a cycle only if a philosopher holds X while waiting on Y. The non-blocking select doesn't *wait* for Y — it tries once and falls through. **If you don't wait, you don't contribute an edge to the deadlock graph.** When Phase A's select fails its non-blocking try, you give back X (left) before doing the blocking `<-rightCh` of Phase B. So no thread is ever holding-while-blocking-on a chopstick that's part of a cycle.

That's why `-race` stays clean here too: the graph TSan/race-detectors care about — `holds X while blocking on Y` — has no edges at all under try-backoff.

### The livelock risk (and why we don't actually see it)

If every philosopher executes Phase A in lockstep — grabs left, finds right held, releases left — then executes Phase B in lockstep — grabs right, finds left held, releases right — and loops, no progress is ever made. That's livelock: no thread is *blocked*, but no thread *progresses* either. Distinct from deadlock; harder to detect with naive analyzers.

In practice, Go's channel FIFO + `time.Sleep` jitter from the surrounding `think` calls + scheduler noise breaks the lockstep. 50/50 stress runs were clean. But the textbook fix — and what production code should add — is a **randomized sleep at the end of the loop body** (e.g., `time.Sleep(time.Duration(rand.Intn(2)) * time.Millisecond)`), which guarantees no two philosophers execute in lockstep beyond a few iterations.

### Compared to the odd/even ring

| | Odd/even ring | Try-and-back-off |
|---|---|---|
| Cycle in lock-acquire graph | Absent (P1, P3 reverse direction) | Absent (no philosopher holds-while-blocking) |
| Deadlock-free | yes | yes |
| Livelock-free | yes | technically no — relies on timing noise |
| Per-meal CPU work | 2 channel ops + 2 channel ops to release | up to 6 channel ops (Phase A: recv + try-recv + send-back) before success |
| Symmetric across philosophers? | no — even/odd asymmetric | yes — every philosopher runs the same code |
| C++ analogue | asymmetric strategy (philosopher N−1 reverses) | `std::scoped_lock{a, b}` |

In benchmarks both came in around 240–270ms for the standard run; well within timing noise of each other. The interesting differences would only show under adversarial scheduling — which we'd need to construct to see.

---

## Deep dive: the algorithm — odd/even ring

The algorithmic strategy was straightforward; I want to be able to reconstruct *why* it works without rereading.

### What each philosopher does

- pid 0 (even): acquires `chopsticks[0]` (left) first, then `chopsticks[1]` (right).
- pid 1 (odd):  acquires `chopsticks[2]` (right) first, then `chopsticks[1]` (left).
- pid 2 (even): acquires `chopsticks[2]` (left) first, then `chopsticks[3]` (right).
- pid 3 (odd):  acquires `chopsticks[4]` (right) first, then `chopsticks[3]` (left).
- pid 4 (even): acquires `chopsticks[4]` (left) first, then `chopsticks[0]` (right).

The lock-acquire graph (edges: "thread holds X while requesting Y"):

```
0:  cs0 → cs1
1:  cs2 → cs1
2:  cs2 → cs3
3:  cs4 → cs3
4:  cs4 → cs0
```

There is **no cycle** in this graph. The "first" sets are `{cs0, cs2, cs4}` and the "second" sets are `{cs0, cs1, cs3}`. They overlap on `cs0` (P0 takes it first, P4 takes it second), but that single overlap doesn't close a cycle by itself — to deadlock you need every philosopher to be holding their first while waiting for their second, and the second-of-A must be the first-of-B for some chain of A→B→C→…→A. It just doesn't close.

Compare to the **naive** strategy where everyone takes left first:

```
0: cs0 → cs1
1: cs1 → cs2
2: cs2 → cs3
3: cs3 → cs4
4: cs4 → cs0     ← closes the cycle: P4 holds cs4, waits cs0; P0 holds cs0, waits cs1; ... ; circle.
```

Cycle. Deadlock-prone (in practice, with N=5 and any contention, it deadlocks within seconds).

### Why the C++ footman has a cycle but the Go asymmetric doesn't

This is the cross-language comparison that's worth internalizing. Both strategies *do not deadlock in practice*. But they prevent it differently:

| | C++ footman | Go asymmetric (this) |
|---|---|---|
| Lock-acquire graph | Has a cycle | **No cycle** |
| Why no actual deadlock | Pigeonhole: ≤ N−1 contenders cap → no full cycle ever forms | Cycle structurally absent |
| Static analyzer happy? | **No** — TSan flags it | **Yes** — `-race` clean |

The Go run is "rigorous": the analyzer's static check passes because the property it checks (graph acyclic) is genuinely true. The C++ run was passing-but-flagged: TSan was right that the graph has a cycle; the program was right that the cycle never closed at runtime. Different rigor.

### The channel-as-token pattern

Each chopstick is `chan struct{}` with capacity 1, primed with one token at startup:

```go
s.chopstickChs[i] = make(chan struct{}, 1)
s.chopstickChs[i] <- struct{}{}    // the chopstick "is here"
```

Acquire = receive the token (`<-ch`). Release = send the token back (`ch <- struct{}{}`).

This is **Pattern A** from the Go readers-writers writeup: token-as-resource, recv = take, send = put back. It's the natural fit when the channel logically *carries the resource*. (Pattern B — empty channel, send=acquire, recv=release — wouldn't work here because there's nothing for the channel to carry.)

Mental shortcut: if you can read the channel value as "the chopstick is here," recv-to-take and send-to-put-back follows naturally.

---

## Final results

| Strategy | Single run time | Meals per philosopher | Spread | 50-run race stress |
|---|---|---|---|---|
| `eat` (odd/even ring) | 274 ms | 50 50 50 50 50 | 0 | 50/50 ✓ |
| `eatTryBackoff` (try-and-back-off) | 242 ms | 50 50 50 50 50 | 0 | 50/50 ✓ |

- **50-run stress under `-race`: 50/50 for both strategies.** No deadlock, no race-detector warnings, no invariant panics.
- Try-backoff was ~12% faster on this hardware — within timing noise. Real differences would only appear under adversarial scheduling.
- Spread of 0 is **meal-cap-bound, not real fairness** — every philosopher hits 50 meals before another falls behind. To expose actual fairness differences you'd switch to a time-bounded run (eat as much as you can in X ms) and see whether the meal counts diverge.

---

## Cross-references

- `sync-problems/cpp/dining_philosophers/dining-philosophers-learnings.md` — the C++ companion. The footman + TSan deep-dive there is the direct counterpart to the "no cycle in the graph" result here.
- `sync-problems/go/readers_writers/readers-writers-learnings.md` — the channel-as-semaphore Pattern A vs Pattern B taxonomy that explains the chopstick-token shape used here.
- `sync-problems/go/producer_consumer/producer-consumer-learnings.md` — `chan struct{}` semantics (cap 0 vs cap 1 vs `close` as broadcast). Same primitive, different role.
- `sync-problems/docs/recap.md` — the cross-cutting "primitive choice shapes where complexity lives" theme.

---

## Next moves

- [ ] **Add a footman-channel variant** as a sibling — `make(chan struct{}, N-1)` primed with N-1 tokens; each philosopher acquires from the footman, then their two chopsticks in *naive* order (left then right). Predict: passes `-race` because the actual lock-acquire graph still has a cycle, same as C++ — but Go's race detector doesn't do lock-order analysis the way C++ TSan does, so it stays quiet either way. **Worth checking**.
- [ ] **Add the naive deadlock variant** — everyone takes left first. Run it once; expect the Go runtime to print `fatal error: all goroutines are asleep - deadlock!` within seconds. That's Go's killer feature: deterministic deadlock detection from the runtime, no analyzer needed.
- [ ] **Time-bounded run** to expose fairness numerically. Replace the meal count with `for time.Now().Before(deadline)`, run for 500 ms, check whether the spread is still 0 (it shouldn't be — asymmetric ring is deadlock-free but not fairness-guaranteed).
- [ ] **Rename the helpers**. `getEvenIdxChopstickCh` doesn't refer to even-indexed chopsticks; it refers to "the chopstick this philosopher takes first if their pid is even." Inline the logic into `eat()` or rename to `firstAcqCh` / `secondAcqCh` for clarity. (My future self will thank me.)
- [ ] **Sabotage experiments to cement the lessons:**
  - Comment out `s.Init()` → runtime deadlock on first recv from a nil channel. Watch Go's "all goroutines asleep" message fire.
  - Change `make(chan struct{}, 1)` to `make(chan struct{})` (cap 0) → a primed `<-` blocks forever because there's no receiver yet to rendezvous with. Different deadlock from the nil case; readers-writers writeup covers this exact distinction.
  - Replace one of the asymmetric-ordering helpers so everyone takes left first. Watch the textbook deadlock surface.
  - Swap `chan struct{}` for `chan bool` everywhere → memory layout changes (1 byte per item instead of 0), behavior identical. Worth doing once just to feel that `struct{}` is the only difference.
