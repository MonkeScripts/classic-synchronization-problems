# Feedback Checklist — What to Tell Me When You Ask for Review

When you paste code and ask "am I on the right track?", the more of this you include, the better my feedback will be.

## The template

```
Problem: [e.g., Dining Philosophers]
Language: [C++ / Rust / Go]
Approach: [e.g., asymmetric lock ordering; odd philosophers pick right first]

Scenario: [e.g., 5 philosophers, each eats 100 times, random think/eat durations]

What I tried:
- [thing 1]
- [thing 2]

What I observed:
- Ran 1000 iterations under `-race` — clean.
- No deadlock in 60s stress test with N=5.
- BUT: philosopher 2 eats noticeably more often than philosopher 4. Why?

Specific question:
- Is my use of `std::scoped_lock` here actually giving me deadlock avoidance,
  or am I just getting lucky?

[paste code]
```

## What makes this better than "is this right?"

- **You name what you're worried about.** I can then verify *that specific thing* instead of general code review.
- **You show you've tested.** I know what level I'm reviewing at.
- **You give measurements.** I can spot "philosopher 2 eats more" as potentially a fairness bug.

## Things I'll check when you ask

Across all problems:
- [ ] Any obvious data race (unsynchronized shared mutable state)?
- [ ] Any unbounded wait (missing signal, wrong predicate)?
- [ ] Any lock-order-inversion (deadlock potential)?
- [ ] Resource leak (forgotten `unlock`, unclosed channel, leaked goroutine)?
- [ ] Idiomatic for this language?

Per problem:
- **Producer-consumer**: wait under predicate (not `if`)? buffer invariant held across unlock?
- **Readers-writers**: can writers starve? did you reinvent the RWLock when the language gives you one?
- **Barrier**: reusable correctly? no "one lap ahead" bug? what happens when N changes mid-round (usually: nothing good)?
- **Dining philosophers**: which anti-deadlock strategy and did you implement it correctly? fairness?
- **Barbershop**: customer/barber signal counts match? balked customers actually leave cleanly?

## When NOT to ask

You'll learn more by doing these yourself first:
- Running your own stress test (see `testing-for-bugs.md`)
- Reading your language's standard library docs for the primitive you're using
- Trying one failure mode deliberately (e.g., "what if I remove this lock?")

Then come to me with "I tried X and got Y — why?"
