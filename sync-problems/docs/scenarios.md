# Scenarios — Concrete Framings for Abstract Problems

Each textbook problem becomes easier to reason about (and more fun to implement) when you give it a concrete skin. Pick one per problem, or invent your own. The scenario shouldn't change the *structure* — it should just make the state transitions vivid.

---

## 1. Producer-Consumer

### Abstract
N producers push to a bounded buffer of size K, M consumers pop from it.

### Scenario A: Log aggregator
- Producers: worker threads emit log lines (strings with a timestamp).
- Buffer: ring of size K.
- Consumer: single writer thread flushes batches to a file (simulate with a slow `sleep`).
- Vary: burst producers vs steady consumer — when does the buffer fill?

### Scenario B: Web scraper pipeline
- Producers: fetchers that download pages (simulate with random sleep 50–200ms).
- Buffer: URLs/HTML waiting to be parsed.
- Consumers: parsers that extract links (fast) and push new URLs back in — **this closes the loop** and gets interesting.
- Watch for: when does the system naturally stall?

### Scenario C: Image thumbnail generator
- Producers: file watchers emit paths of new images.
- Consumers: generate thumbnails (CPU-bound).
- Good for comparing: does more consumers = more throughput? (Measure.)

### What to actually measure
- Throughput (items/sec) as you vary K.
- Fairness: does each consumer get roughly equal work?
- What happens if a consumer panics/throws?

---

## 2. Readers-Writers

### Abstract
Many readers OR one writer at a time, on a shared data structure.

### Scenario A: In-memory key-value cache
- Data: `HashMap<String, String>` (or equivalent).
- Readers: `get(key)` — many concurrent.
- Writers: `set(key, value)` — exclusive.
- Compare: plain mutex vs RWLock vs `shared_mutex`. When does RWLock actually win?

### Scenario B: Config service
- One "config" struct with many fields.
- Many threads read current config frequently.
- Rare writer pushes a new config version.
- Variant: implement with a **reader-preferring** lock, then a **writer-preferring** one, then a **fair** one. Observe starvation.

### Scenario C: Order book (finance-flavored)
- Data: sorted buy/sell orders.
- Readers: "what's the best bid/ask?" (many per millisecond).
- Writers: insert/cancel order.
- Real lesson: when reads are *fast*, RWLock overhead can make plain mutex faster. Measure!

### What to actually measure
- Read throughput vs writer count.
- Writer wait time distribution (is any writer starved?).
- Compare the three lock flavors on the *same* workload.

---

## 3. Barrier

### Abstract
N threads must all reach a point before any proceeds. Reusable means "repeatable across rounds."

### Scenario A: Parallel iterative solver (Jacobi)
- Partition an array across N threads.
- Each round: compute update, **barrier**, swap buffers, **barrier**, repeat.
- Natural fit — this is literally how HPC code uses barriers.

### Scenario B: Turn-based game simulation
- N player AIs each compute their move for turn T.
- Barrier: wait for all moves.
- Apply moves atomically, **barrier**, start turn T+1.
- Good for catching the "one lap ahead" bug — does any player ever start turn T+1 while another is still on T?

### Scenario C: Race start
- N runners do `warmup()` then must all cross the start line together.
- `barrier.wait()` is the starting gun.
- `race()` then `barrier.wait()` to reset for next heat.
- Fun to visualize with prints including thread IDs and timestamps.

### What to actually measure
- Print thread ID + round number on both sides of the barrier. No thread should print round R+1 before all threads printed round R.
- Try with 2, 10, 100 threads — does your impl scale?

---

## 4. Dining Philosophers

### Abstract
N philosophers around a table, N chopsticks between them. Each needs both neighbors' chopsticks to eat.

### Scenario A: Database transactions acquiring row locks
- "Philosophers" = transactions.
- "Chopsticks" = row locks.
- Deadlock = the classic "T1 holds row A wants B; T2 holds B wants A" — this *is* dining philosophers with N=2.
- Resolve with: lock ordering (always acquire lower-ID first) — the **asymmetric solution**.

### Scenario B: Printer/scanner resource allocation
- 5 office workers, each periodically needs printer+scanner together.
- Only one printer, one scanner.
- Classic deadlock setup.

### Scenario C: Literal dining philosophers
- Just do it. Print `P0 is eating`, `P0 put down chopsticks`, etc.
- Verify: no two adjacent philosophers ever eat simultaneously.
- Verify: every philosopher eventually eats (no starvation).

### Solutions to implement (all three at minimum)
1. **Naive** (demonstrates deadlock — leave it in, annotated).
2. **Asymmetric** / resource hierarchy — philosopher N-1 picks up right first.
3. **Limited eaters** — use a "footman" semaphore allowing at most N-1 to try eating.
4. **Tanenbaum** — track philosopher states (THINKING/HUNGRY/EATING) with per-philosopher semaphores.
5. **Chandy-Misra** (stretch) — tokens/tickets for each chopstick.

### What to actually measure
- Does any philosopher *never* eat in 10,000 rounds? (starvation)
- Does the whole system freeze? (deadlock)
- Throughput: meals/second across all philosophers.

---

## 5. Barbershop (Sleeping Barber)

### Abstract
1 barber, N waiting chairs. Barber sleeps when no customers. Customers leave if full.

### Scenario A: Customer service phone line
- Barber = agent, chairs = hold queue slots.
- "Balk" = caller hears busy signal and hangs up.
- Easy to visualize.

### Scenario B: Connection pool / thread pool
- Barber = worker thread.
- Chairs = queued tasks.
- Tasks submitted when queue full are rejected (like `ThreadPoolExecutor` with `AbortPolicy`).
- Real system-design parallel.

### Scenario C: Food truck with N-person line
- N = 5. Customers arrive randomly (poisson-ish). Service time varies.
- Measure: average wait, how many balked.

### Variations worth implementing
1. **Single barber** — the textbook version.
2. **Multiple barbers** — M barbers, N chairs. Harder: barbers need to coordinate.
3. **FIFO guarantee** — customers served in arrival order (not trivial with semaphores!).
4. **Priority customers** — VIP customers jump the queue (shows why fairness is hard).

### What to actually measure
- Was any customer "lost" (signaled but not served)? This is the classic barbershop bug.
- Was any customer served out of order?
- Barber idle time vs customer wait time as arrival rate changes.

---

## Common scenario-design principles

1. **Make state visible.** Print transitions with timestamps and IDs. Your eyes are a good race detector.
2. **Vary rates.** A bug hides when all operations take 100ms. Try 1ms producers + 100ms consumers, then flip it.
3. **Run it a LOT.** 10,000 iterations with randomized sleeps catches more than 10 iterations with fixed sleeps.
4. **Compare across languages.** If your Go version behaves differently from your C++ version, one of them has a bug (or a different memory model assumption).
