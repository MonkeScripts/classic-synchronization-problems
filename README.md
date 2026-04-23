# Classical Synchronization Problems — Learning Workspace

A personal workspace for implementing classical synchronization problems in **C++**, **Rust**, and **Go**, based on CS3211 L9.

## Philosophy

The point is **not** to collect working solutions. The point is to feel how each language's concurrency model shapes the solution — where locks feel natural, where channels feel natural, where the type system helps you, and where you have to be careful.

For each problem, try to implement it in **at least two different ways per language** when possible. The second attempt usually teaches more than the first.

## Problems

| Problem | Core Concept | Key Pitfall |
|---|---|---|
| Producer-Consumer | Bounded buffer, condition signaling | Lost wakeups, spurious wakeups |
| Readers-Writers | Shared vs exclusive access | Writer starvation |
| Barrier | Phase synchronization | Reusability / "one lap ahead" bug |
| Dining Philosophers | Resource ordering | Deadlock, livelock, starvation |
| Barbershop | Rendezvous + bounded queue | Mismatched signals, lost customers |

## Current state of templates

| Problem | C++ | Rust | Go |
|---|---|---|---|
| Dining Philosophers | ✅ template | ✅ template | ✅ template |
| Barrier | ✅ template | ✅ template | ✅ template |
| Barbershop | ✅ template | ✅ template | ✅ template |
| Producer-Consumer | ✅ template | ✅ template | ✅ template |
| Readers-Writers | ✅ template | ✅ template | ✅ template |

✅ = full file ready to fill in TODOs

All templates follow the same pattern: constants → shared state → invariant checker → (jitter where relevant) → TODO blocks → fairness/correctness output. `docs/sketches.md` remains as a reference for alternate shapes you might want on a second pass.

## Directory Layout

```
sync-problems/
├── cpp/
│   ├── common/                  # FairMutex, helpers, etc.
│   ├── barrier/
│   ├── dining_philosophers/
│   ├── barbershop/
│   ├── producer_consumer/
│   └── readers_writers/
├── rust/
│   └── ... (same)
├── go/
│   └── ... (same)
└── docs/
    ├── scenarios.md             # Concrete problem framings
    ├── testing-for-bugs.md      # How to actually detect races/deadlocks
    └── feedback-checklist.md    # What to ask me to review
```

## Workflow

1. Read the lecture section for a problem.
2. Pick a **scenario** from `docs/scenarios.md` (or invent one) — gives the abstract problem a concrete shape.
3. Implement in one language. Run it. Stress-test it (see `docs/testing-for-bugs.md`).
4. Paste me the code + what you tried + what you observed. Ask "am I on the right track?"
5. Implement the *same* scenario in the next language. Reflect on what changed.

## What to ask me

Good questions that will get you useful answers:

- "Here's my dining philosophers in Rust using `Arc<Mutex<_>>` — does this deadlock? I ran it 1000 times and it didn't, but I'm not sure that proves anything."
- "My Go barrier works for N=4 but hangs for N=100. Here's the code — what ordering bug am I making?"
- "C++: I used `std::condition_variable::wait` without a predicate. What could go wrong?"
- "Compare my three barbershop implementations — which one most honors each language's idioms?"

Less useful questions (you'll learn less):

- "Write me a barrier in Rust." *(Just gives you a solution.)*
- "Is this correct?" *(Too open — try to name what you're worried about.)*

## Build quickstart

- **C++**: each problem has a `CMakeLists.txt`. `mkdir build && cd build && cmake .. && make`
- **Rust**: each problem is its own cargo crate. `cargo run --release`
- **Go**: each problem is a Go module. `go run .` or `go test -race ./...`

**Always** run Go with `-race` and Rust/C++ with thread sanitizer when debugging. See `docs/testing-for-bugs.md`.
# classic-synchronization-problems
