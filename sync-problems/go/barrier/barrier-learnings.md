# Barrier in Go: Mistakes & Learnings

Companion to `cpp/barrier/barrier-learnings.md`. The Go side has two variants:

- `go/barrier/main.go` — two `*sync.WaitGroup`s with the snapshot-and-swap pattern (the slide-23 shape).
- `go/barrier_cond/main.go` — `sync.Mutex` + `*sync.Cond` + generation counter (the classic).

The cond version is shorter and cleaner. The WG version is where the real Go-specific learning happened — the surface-level "use two WaitGroups like two semaphores" looks like it should work, and Go's stdlib quietly tells you it doesn't.

---

## The journey at a glance

| Stage | Mistake | What it taught me |
|---|---|---|
| 1 | `entry_wg: wg.Add(expected)` inside the struct literal | Composite literals can only set fields to *values*; `Add` returns nothing, and there was no `wg` in scope anyway |
| 2 | `wg.Done(); wg.Wait(); wg.Add(1)` to reuse a WaitGroup | Go panics: *"sync: WaitGroup is reused before previous Wait has returned."* WaitGroup is single-use by design |
| 3 | Fields typed as `sync.WaitGroup` (value) | Can't compare values for identity; can't legally copy a WG (`noCopy` lint). Swap-pattern requires `*sync.WaitGroup` |
| 4 | Copy-paste typo: `if b.exit_wg == wg` should be `wg2` | The snapshot variable matters — wrong one means the swap never fires, round 2 panics |
| 5 | Skipped to cond + generation | Fewer moving parts. The pattern that's actually idiomatic in Go for a reusable barrier |

---

## Mistake 1: struct-literal init with method calls

### What I wrote
```go
return &MyBarrier{
    expected: expected,
    entry_wg: wg.Add(expected),
    exit_wg:  wg.Add(expected),
}
```

### Why it doesn't compile
- `wg` doesn't exist in scope.
- `(*sync.WaitGroup).Add` returns nothing — its type is `func(int)`, not `func(int) sync.WaitGroup`.
- Composite literals take *values* for fields, not statements that mutate something.

### The fix
Build the WG first, then assign:
```go
func freshWG(n int) *sync.WaitGroup {
    wg := &sync.WaitGroup{}
    wg.Add(n)
    return wg
}

return &MyBarrier{
    expected: expected,
    entry_wg: freshWG(expected),
    exit_wg:  freshWG(expected),
}
```

### The bigger lesson
Go's composite literals are intentionally *declarative* — you describe the shape of the value, not a recipe. Anything that needs a side effect (like `Add`) belongs in a constructor, not a literal.

---

## Mistake 2: trying to reuse a `sync.WaitGroup`

### What I wrote
```go
func (b *MyBarrier) ArriveAndWait() {
    b.entry_wg.Done()
    b.entry_wg.Wait()
    b.entry_wg.Add(1)   // <-- prep for next round
    b.exit_wg.Done()
    b.exit_wg.Wait()
    b.exit_wg.Add(1)
}
```

### Why it's wrong

The Go runtime panics with:
```
sync: WaitGroup is reused before previous Wait has returned
```

The stdlib spec says:
> *"new Add calls must happen after all previous Wait calls have returned."*

Concretely: when the counter hits 0, all `Wait` calls become eligible to return — but they haven't physically returned yet. If thread A returns from `Wait` and races to `Add(1)` before thread B's `Wait` has unwound, the runtime catches the violation.

### The sentence that named the primitive

> *"`sync.WaitGroup` is a count-down latch, not a cyclic barrier."*

Latches are single-use by design. They're for "wait for these N events, then we're done." For "wait for these N, then reset, then wait for the next N, ..." you need a different shape.

### The two ways out
1. **Allocate a fresh WG each round** (the snapshot-and-swap pattern below).
2. **Don't use WaitGroup** — use mutex + cond + generation, which is what `barrier_cond/` does.

---

## Mistake 3: fields as `sync.WaitGroup` instead of `*sync.WaitGroup`

### What I wrote
```go
type MyBarrier struct {
    entry_wg sync.WaitGroup
    exit_wg  sync.WaitGroup
}
```

…then later:
```go
b.mu.Lock()
wg := b.entry_wg          // copies the WG
b.mu.Unlock()
// ...
if b.entry_wg == wg { ... }  // doesn't even compile
```

### Two problems collapsed into one

1. **Copying is forbidden.** `sync.WaitGroup` carries a `noCopy` marker. `go vet` flags any pass-by-value or assignment. Even if it didn't, you'd be copying internal counter state, which is a guaranteed bug.
2. **Identity comparison needs pointers.** The swap pattern depends on "is the WG I snapshotted *still* the one in the field?" — that's `==` on pointers. Two `WaitGroup` values can't be compared with `==` at all.

### The fix
Make them pointers, allocate fresh ones via `freshWG`, and the snapshot is just a pointer copy.

### The bigger lesson
"Don't copy this type" → store as pointer. Same rule for `sync.Mutex`, `sync.Cond`, `bytes.Buffer`, anything carrying lock-like state. Go conveys this via `noCopy` and you should listen.

---

## Mistake 4: copy-paste typo in the swap check

### What I wrote
```go
b.mu.Lock()
if b.exit_wg == wg {   // <-- wg is the entry snapshot, not exit
    b.exit_wg = freshWG(b.expected)
}
b.mu.Unlock()
```

### What goes wrong
`wg` is the Phase-1 snapshot of `entry_wg`. `b.exit_wg` was never that pointer, so `b.exit_wg == wg` is *always* false. The exit gate never gets swapped. Round 2's Phase 2 hits the WaitGroup-reuse panic because the same `*sync.WaitGroup` is being `Done`/`Wait`/`Done`'d across rounds.

### The fix
```go
if b.exit_wg == wg2 { b.exit_wg = freshWG(b.expected) }
```

### The bigger lesson
Symmetric two-phase code (`wg/wg2`) is a copy-paste hazard. Worth re-reading the swap block as a unit instead of trusting muscle memory. The stress-test loop under `-race` catches this on round 2 — round 1 looks fine because the entry-gate swap *did* happen.

---

## The pattern, distilled

For any "broadcast that must be reusable" in Go, the shape is:

```
   <snapshot the current gate under lock>
   <do the wait>
   <CAS-style swap: if I'm the first one past, install a fresh gate>
```

This is the pattern in both:
- WG variant: snapshot `*sync.WaitGroup`, swap with `freshWG` after `Wait`.
- Channel-close variant (not implemented here): snapshot `chan struct{}`, swap with `make(chan struct{})` after the close fires.

The cond + generation variant skips the swap entirely because **the generation counter is the gate identifier** — a single `uint64` does the work two `*sync.WaitGroup`s do in the WG variant.

---

## Why I'd pick the cond version in real code

| Variant | Lines of meaningful code | Concepts you have to hold | Failure mode if you get it wrong |
|---|---|---|---|
| Two `*sync.WaitGroup`s + swap | ~25 | snapshot, swap, identity-CAS, freshWG helper | runtime panic on round 2 |
| `sync.Cond` + generation | ~10 | gen-snapshot, predicate-loop | almost none — `cv.Wait(lock, pred)` shape is hard to get wrong |

The semaphore-like shape *looks* familiar (it's slide 23) but Go's `sync.WaitGroup` isn't a semaphore — it's a single-use latch with a `noCopy` marker, and forcing it into "barrier" requires the swap dance. Cond + generation is the shape Go's `sync` package was built for.

---

<!-- Add stages below as you hit and understand new bugs. -->
