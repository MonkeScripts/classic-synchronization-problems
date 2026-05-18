# EXAM CHEAT SHEET — CS3211 Classical Synchronization

> Print and staple. One page per problem. Open book, physical paper. Use the deep docs (`EXAM_GUIDE_PER_PROBLEM.md`, `EXAM_GUIDE_PER_LANGUAGE.md`, `IMPLEMENTATION_PATTERNS.md`) for pre-exam study; this sheet is the exam-time reminder.

**Pages:**
1. Triage — find the pattern
2. Producer-Consumer
3. Readers-Writers
4. Barrier
5. Dining Philosophers
6. Barbershop
7. H2O (Water Factory)
8. FIFO Semaphore
9. Search-Insert-Delete
10. Bridge Crossing
11. Language quick-reference
12. Decision tables + universal gotchas

---

## Page 1 — TRIAGE

### Tree 1: What pattern? (walk top to bottom)

```
START: How do the threads/tasks RELATE?

(A) Sharing mutable state, coordinating access
   ├ At most one at a time?            → A1 EXCLUSION (mutex)
   ├ Up to N at a time (N>1)?           → A2 BOUNDED-N (counting sem)
   ├ Many readers OR one writer?        → A3 RW (Page 3)
   ├ 3+ asymmetric roles?               → A4 N-ROLE LIGHTSWITCH (SID p9, Bridge p10)
   └ Wait until predicate is true?      → A5 PREDICATE-WAIT (cv + while)

(B) Passing data/messages between actors
   ├ 1→1, single value, fire-and-forget?  → B1 ONESHOT
   ├ Many→1, queued?                       → B2 MPSC (PC p2)
   ├ Many→many, work-stealing pool?         → B3 MPMC (mpsc + Arc<Mutex<Receiver>>)
   ├ 1→many, every event?                  → B4 BROADCAST
   ├ 1→many, latest snapshot only?         → B5 WATCH
   └ Producer waits for consumer to ack?  → B6 REQUEST-RESPONSE (Barbershop p6)

(C) All actors must reach a sync point
   ├ Once, then never again?            → C1 LATCH (sync.WaitGroup)
   └ Reusable across phases?             → C2 BARRIER (cond+generation, p4)

(D) One actor orchestrates a protocol
   ├ Workers submit themselves?         → D1 SUBMIT-YOURSELF DAEMON (H2O p7)
   └ Driver picks who works each round? → D2 ADDRESSED-WORKER DAEMON (Smokers)
```

### Tree 2: Pattern × Language → Primitive

| Pattern | C++ | Rust sync | Rust tokio | Go |
|---|---|---|---|---|
| A1 Exclusion | `std::mutex` | `Arc<Mutex<T>>` | `tokio::sync::Mutex` (only across `.await`; else std) | `sync.Mutex` or `chan struct{}` cap 1 |
| A2 Bounded-N | `std::counting_semaphore<N>` | `parking_lot::Semaphore` | `tokio::sync::Semaphore` | `chan struct{}` cap N |
| A3 RW | `std::shared_mutex` | `RwLock` | `tokio::sync::RwLock` | `sync.RWMutex` |
| A4 N-role | mtx + counter + N×bin sem | same | same | mtx + cv + state struct |
| A5 Predicate-wait | `cv.wait(lk, pred)` | `cv.wait_while(g, !pred)` | `Notify` + Mutex re-check | `for !pred { cv.Wait() }` |
| B1 Oneshot | promise/future | `mpsc` (cap 1) | `tokio::sync::oneshot` | `make(chan T)` rendezvous |
| B2 MPSC | hand-rolled cv | `std::sync::mpsc` | `tokio::sync::mpsc` | `make(chan T, N)` |
| B3 MPMC | hand-rolled | `crossbeam-channel` | `Arc<tokio::sync::Mutex<Receiver>>` | channels are MPMC natively |
| B4 Broadcast | hand-rolled | hand-rolled | `tokio::sync::broadcast` | `close(done)` for cancel |
| B5 Watch | n/a | hand-rolled | `tokio::sync::watch` | hand-rolled |
| B6 Req-resp | hand-rolled | hand-rolled | `mpsc<oneshot::Sender<()>>` | `chan chan T` |
| C1 Latch | `std::latch` | `Arc<Once>` | `Notify` + atomic | `sync.WaitGroup` |
| C2 Barrier | `std::barrier` | hand-rolled (cond+gen) | `tokio::sync::Barrier` | hand-rolled (cond+gen) |
| D1 Submit-self daemon | n/a | n/a | `mpsc<Request{go,done}>` | `chan chan T` |
| D2 Addressed daemon | n/a | n/a | N × `mpsc<()>` + shared done | N × per-worker chans |

### Tree 3: Orthogonal axes — decide each

| Axis | Options | Trigger |
|---|---|---|
| **Blocking** | block / try / timeout / **balk** / lossy | spec says "leaves" / "returns error" / "with timeout" / "drops oldest" |
| **Fairness** | none / FIFO / anti-starvation turnstile / handoff | spec says "fair" / "starve-free" / "in arrival order" |
| **Shutdown** | drop sender / close-broadcast / release-N / sentinel | depends on primitive |
| **Consistency** | strict / "may miss in-flight" / snapshot / latest-only | implied by which role pairs are compatible |

### Hot-keyword lookup

| Spec says... | → |
|---|---|
| "balks" / "leaves" / "returns error" / "queue full → walks away" | balk semantics |
| "may starve" / "starve-free" / "fair" / "in arrival order" | FIFO / turnstile |
| "drops oldest" / "limited buffer" | lossy (broadcast Lagged) |
| "with timeout" / "after Xms" | timeout |
| "exclusive" / "no concurrent" | A1 / A4 forbidden pair |
| "subscribe" / "publish" / "stream" | B4 broadcast |
| "latest" / "current" / "snapshot" | B5 watch |
| "agent" / "manager" / "coordinator" | D1 / D2 daemon |
| "phase" / "round" | C2 barrier |

---

## Page 2 — PRODUCER-CONSUMER

**Rules:** N producers push to bounded buffer (cap K). M consumers pop. Producers block on full; consumers block on empty. `close()` → producers bail immediately; consumers drain THEN bail.

**Invariants:**
- `0 ≤ size ≤ K` always.
- `produced == consumed` once channel drained.
- After `close()`: no new pushes accepted.

**English algorithm (predicate-wait variant):**
```
push(x):
  lock; wait while !(size<K OR closed)         # progress OR give-up disjunct
  if closed: unlock; return false
  queue.push(x); notify_one(not_empty); unlock; return true

pop():
  lock; wait while !(size>0 OR (closed AND size==0))
  if size==0: unlock; return None              # closed & drained
  x = queue.pop(); notify_one(not_full); unlock; return x

close():
  lock; closed = true; unlock
  notify_all(not_full); notify_all(not_empty)
```

**Code skeleton (C++ — predicate overload):**
```cpp
bool push(T item) {
    std::unique_lock lk{mu_};
    not_full_.wait(lk, [&]{ return q_.size() < cap_ || closed_; });
    if (closed_) return false;
    q_.push_back(std::move(item));
    not_empty_.notify_one();
    return true;
}
```

**Top mistakes:**
1. **`if` instead of `while`** for predicate → spurious wakeup admits with full/empty buffer.
2. **Symmetric bail** — push and pop bail the same way. WRONG: push bails on closed; pop must drain first (`closed && size==0`).
3. **Notify wrong cv side** after action — push `notify_empty`; pop `notify_full`.
4. **Forget `notify_all` on close** — parked threads stuck forever.

**Cross-language:**
- **Go**: just `make(chan T, K)` + `close(chan)`. Channel IS the buffer. Closer-goroutine pattern for multi-producer.
- **Rust tokio**: `tokio::sync::mpsc::channel(K)`. `tx.send().await` backpressures naturally. `drop(tx)` closes.
- **Rust sync**: `std::sync::mpsc::sync_channel(K)`. `recv()` returns `Result<T, RecvError>`.

---

## Page 3 — READERS-WRITERS

**Rules:** Many readers OR one writer. Three solutions of increasing sophistication.

**Invariants:**
- `readers > 0 ⇒ writers == 0`.
- `writers > 0 ⇒ readers == 0 && writers == 1`.

**English algorithms:**

```
LIGHTSWITCH (reader-preference; starves writers):
  Reader:                                    Writer:
    bibo.acquire                               roomEmpty.acquire
    rc++; if rc==1: roomEmpty.acquire           WRITE
    bibo.release                                roomEmpty.release
    READ
    bibo.acquire
    rc--; if rc==0: roomEmpty.release
    bibo.release

NO-STARVE TURNSTILE (writer-fair):
  Reader: turnstile.acquire; turnstile.release    # gate-pass
          [lightswitch as above]
  Writer: turnstile.acquire                       # HOLD across acquire
          roomEmpty.acquire
          WRITE
          roomEmpty.release; turnstile.release
```

**Code skeleton (C++ no-starve):**
```cpp
class KV {
    std::counting_semaphore<1> turnstile_{1}, roomEmpty_{1}, bibo_{1};
    int rc_ = 0;
public:
    string get(...) {
        turnstile_.acquire(); turnstile_.release();
        bibo_.acquire(); ++rc_;
        if (rc_==1) roomEmpty_.acquire();
        bibo_.release();
        // READ
        bibo_.acquire(); --rc_;
        if (rc_==0) roomEmpty_.release();
        bibo_.release();
    }
    void set(...) {
        turnstile_.acquire(); roomEmpty_.acquire();
        // WRITE
        roomEmpty_.release(); turnstile_.release();
    }
};
```

**Top mistakes:**
1. **Compound-op race** — `--rc; if (rc==0) sem.release()` is THREE ops. Two threads can both observe 0 and both release. Mutex must wrap the whole sequence.
2. **Lightswitch starves writers** under sustained reader load. Add turnstile.
3. **`std::counting_semaphore` no FIFO guarantee** — turnstile gives "starve-free under fair primitive," not strict FIFO. ~85% pass rate vs Go/tokio's 100%. THE marquee finding.

**Cross-language:**
- Go: `sync.RWMutex` (writer-preference built in). For hand-rolled, channels-as-semaphores.
- Rust tokio: `tokio::sync::RwLock` (FIFO) or hand-rolled with `Semaphore` (FIFO).

---

## Page 4 — BARRIER (reusable, cyclic)

**Rules:** N threads must all reach the barrier before any proceeds. **Reusable across rounds** (≠ `sync.WaitGroup` which is single-shot).

**Invariants:**
- No thread enters round R+1 while any thread is still in round R.
- All N threads release together, then all park together for next round.

**English algorithm (cond + generation):**
```
arrive_and_wait():
  lock
  gen = state.generation                     # snapshot before incrementing
  state.count++
  if state.count == expected:
      state.count = 0
      state.generation++                     # advance generation
      cv.notify_all()
      unlock; return
  while state.generation == gen:             # wait until gen advances
      cv.wait(lock)
  unlock
```

**Code skeleton (Go):**
```go
type B struct { exp,cnt int; gen uint64; mu sync.Mutex; cv *sync.Cond }
func (b *B) ArriveAndWait() {
    b.mu.Lock(); defer b.mu.Unlock()
    gen := b.gen
    b.cnt++
    if b.cnt == b.exp {
        b.cnt = 0; b.gen++
        b.cv.Broadcast()
        return
    }
    for b.gen == gen { b.cv.Wait() }
}
```

**Top mistakes:**
1. **`bool ready` instead of generation** → "one-lap-ahead" bug: a fast thread enters round R+1 while slow threads still finishing round R.
2. **Reusing `sync.WaitGroup`** for cyclic barrier → "WaitGroup is reused before previous Wait has returned" panic.
3. **`notify_one` instead of `notify_all`** when threshold met — only one thread wakes, rest stuck.

**Cross-language:**
- C++: `std::barrier` (C++20) or hand-rolled.
- Rust tokio: `tokio::sync::Barrier` (count-down + broadcast internally).
- Rust sync: hand-rolled cond+generation.

---

## Page 5 — DINING PHILOSOPHERS

**Rules:** N philosophers, N chopsticks. Each needs LEFT and RIGHT chopstick to eat. **No deadlock, no starvation.**

**Invariants:**
- No two adjacent philosophers EATING simultaneously.
- Every philosopher eventually eats.

**Strategies (pick one):**

```
1. ASYMMETRIC LOCK ORDER (break the cycle):
   Each philosopher picks lower-numbered chopstick first.
   Philosopher N-1 picks RIGHT first (the only one going other way).
   → Acyclic resource graph → no deadlock. Hold-and-wait OK.

2. FOOTMAN (cap concurrent diners at N-1):
   counting_semaphore footman{N-1}
   eat(): footman.acquire(); pick L; pick R; ...; footman.release()
   → Pigeonhole: N-1 diners can't all hold both chopsticks → no deadlock.

3. STATE-TRACK (Tanenbaum):
   state[i] in {THINKING, HUNGRY, EATING}
   take(i): state[i]=HUNGRY; test(i)              # EATING iff neighbours not eating
            if state[i]!=EATING: cv[i].wait()
   put(i):  state[i]=THINKING; test(left); test(right)   # wake hungry neighbours

4. SCOPED_LOCK (C++ atomic two-mutex):
   std::scoped_lock lk{chop[L], chop[R]};         # internal try-and-back-off
```

**Code skeleton (C++ scoped_lock):**
```cpp
void philosopher(int id) {
    int L = id, R = (id+1) % N;
    while (eating < ROUNDS) {
        std::scoped_lock lk{chop[L], chop[R]};
        // eat — both held; LIFO release at }
    }
}
```

**Top mistakes:**
1. **Symmetric pickup order (everyone L then R)** → all hold L, all wait for R → deadlock.
2. **`lock_guard` for two mutexes sequentially** — no deadlock-safe ordering. Use `scoped_lock` (variadic).
3. **State-track without notifying neighbours** — `put()` must `test(left)` and `test(right)` to wake them.

**Cross-language:**
- Rust: `parking_lot::Mutex` array, asymmetric ordering.
- Go: chopsticks as `chan struct{}` cap-1 with token-as-resource pattern (G2). Footman = `chan struct{}` cap N-1.

---

## Page 6 — BARBERSHOP (sleeping barber)

**Rules:** 1 barber, K chairs. Customer arrives: sit if chair free, balk if full. Barber sleeps when no customers; wakes when one arrives; cuts hair; sleeps again.

**Invariants:**
- At most K customers waiting + 1 being cut.
- Every customer either served or balks (never silently dropped).
- `served + balked == total`.

**English algorithm (mpsc<oneshot> shape):**
```
mpsc::channel(K) carries oneshot::Sender<()>  # the chair queue
                                              # cap K = K chairs

Customer:
  (done_tx, done_rx) = oneshot::channel()
  match chair_tx.try_send(done_tx):
    Ok        → done_rx.await; SERVED
    Err(Full) → BALKED
    Err(Closed) → shop closed

Barber:
  loop {
    match chair_rx.recv().await:
      Some(done_tx): cut_hair(); done_tx.send(())
      None: shop closed → exit
  }
```

**Code skeleton (Rust tokio):**
```rust
let (chair_tx, mut chair_rx) = mpsc::channel::<oneshot::Sender<()>>(K);

// barber:
tokio::spawn(async move {
    while let Some(done_tx) = chair_rx.recv().await {
        sleep(Duration::from_millis(10)).await;       // cut hair
        let _ = done_tx.send(());                      // wake customer
    }
});

// customer:
let (done_tx, done_rx) = oneshot::channel();
match chair_tx.try_send(done_tx) {
    Ok(()) => { done_rx.await.unwrap(); /* served */ }
    Err(TrySendError::Full(_)) => { /* balked */ }
    Err(TrySendError::Closed(_)) => panic!(),
}
```

**Top mistakes:**
1. **Use blocking `send` instead of `try_send`** for customer → backpressure instead of balk. Spec said "if no chair, customer leaves."
2. **`done_tx.send(()).await.unwrap()`** — `oneshot::Sender::send` is **sync**, returns `Result<(), T>`. No await.
3. **Forgetting to drop main's `chair_tx`** before awaiting barber → barber loops forever.

**Multi-barber variant (M doctors / clinic):** Wrap `chair_rx` in `Arc<tokio::sync::Mutex<...>>`, M barber tasks share it. **Tight lock scope**: `let next = { lock; recv().await }` — release BEFORE the consultation, otherwise throughput collapses to 1.

---

## Page 7 — H2O (Water Factory)

**Rules:** H atoms and O atoms arrive. Bond into H₂O = 2H + 1O. Atoms must wait until 2H + 1O are simultaneously present, then all 3 bond together.

**Invariants:**
- At most 2 H and 1 O bonding at any moment.
- `MAX_TOTAL_IN_BOND` reaches 3 (catches "atoms bonding solo without barrier").
- `total_h_bonds == 2 × total_o_bonds`.

**Strategies:**

```
1. SEMAPHORE + BARRIER:
   h_sem(2), o_sem(1), barrier(3)
   hydrogen(): h_sem.acquire().forget(); barrier.wait(); bond_h(); h_sem.add_permits(1)
   oxygen():   o_sem.acquire().forget(); barrier.wait(); bond_o(); o_sem.add_permits(1)
   → semaphores limit; barrier makes all 3 bond simultaneously.

2. DAEMON (submit-yourself):
   Atoms submit (go_tx, done_rx) via mpsc; daemon collects 2H+1O,
   sends () on each go_tx, awaits each done_rx.

3. LEADER ELECTION (oxygen-leads):
   Single oxygen "leader" semaphore (cap 1). O collects 2 H precommits,
   signals all 3, awaits dones, releases leader.
```

**Code skeleton (Rust tokio — daemon):**
```rust
struct Request { go: oneshot::Sender<()>, done: oneshot::Receiver<()> }

async fn daemon(mut h_rx: Receiver<Request>, mut o_rx: Receiver<Request>) {
    loop {
        let h1 = h_rx.recv().await.unwrap();
        let h2 = h_rx.recv().await.unwrap();
        let o  = o_rx.recv().await.unwrap();
        h1.go.send(()).unwrap(); h2.go.send(()).unwrap(); o.go.send(()).unwrap();
        h1.done.await.unwrap(); h2.done.await.unwrap(); o.done.await.unwrap();
    }
}

async fn hydrogen(&self, id: usize) {
    let (go_tx, go_rx) = oneshot::channel();
    let (done_tx, done_rx) = oneshot::channel();
    self.h_tx.send(Request { go: go_tx, done: done_rx }).await.unwrap();
    go_rx.await.unwrap();
    bond_h(id).await;
    done_tx.send(()).unwrap();
}
```

**Top mistakes:**
1. **No barrier in strategy 1** → atoms bond solo (h1 done before h2 arrives). `MAX_TOTAL_IN_BOND` would only reach 1.
2. **Forget `.forget()` on tokio Sem permit** → permit auto-returns at scope end → semaphore re-fills → next round races early.
3. **One oneshot for go-AND-done** → can't reuse single-fire oneshot. Need TWO per atom.

**Variant — Cigarette Smokers (Patil):** addressed-daemon. Agent picks 1 of 3 ingredient-holders per round, sends `()` on smoker_tx[i]. Each smoker on its own dedicated mpsc<()>. Shared mpsc<()> done channel.

---

## Page 8 — FIFO SEMAPHORE

**Rules:** Like a counting semaphore but **strict FIFO wakeup order** (oldest waiter wakes first; cancellation handled).

**Why:** `std::counting_semaphore` and parking_lot don't promise FIFO; even tokio's "approximately FIFO" can break under cancellation.

**Strategies:**

```
1. PREDICATE + TICKET (cv-based):
   state: { permits, next_ticket, now_serving }
   acquire():
     lock; my=next_ticket++; while !(now_serving==my && permits>0): cv.wait
     permits--; now_serving++; cv.notify_all; unlock
   release():
     lock; permits++; cv.notify_all; unlock

2. ONESHOT QUEUE (cleaner):
   state: { permits, waiters: VecDeque<oneshot::Sender<()>> }
   acquire():
     lock; if permits>0 && waiters empty: permits--; return
            else: push (tx,rx); unlock; rx.await
   release():
     lock; if waiters nonempty: pop tx; tx.send(())   # direct hand-off
            else: permits++
```

**Code skeleton (Rust — oneshot queue):**
```rust
pub async fn acquire(&self) {
    let rx = {
        let mut s = self.state.lock().unwrap();
        if s.permits > 0 && s.waiters.is_empty() {
            s.permits -= 1;
            return;
        }
        let (tx, rx) = oneshot::channel();
        s.waiters.push_back(tx);
        rx
    };  // *** drop std::Mutex guard BEFORE await ***
    rx.await.unwrap();
}

pub fn release(&self) {
    let mut s = self.state.lock().unwrap();
    if let Some(tx) = s.waiters.pop_front() {
        let _ = tx.send(());                    // hand permit directly
    } else {
        s.permits += 1;
    }
}
```

**Top mistakes:**
1. **Hold `std::sync::Mutex` across `.await`** → future `!Send`. Block-scope the guard.
2. **Bump permits then notify** instead of direct hand-off → another acquirer can steal the permit, breaking FIFO.
3. **Fast-path takes permit even with waiters queued** → leapfrogging older waiters.

---

## Page 9 — SEARCH-INSERT-DELETE

**Rules (3 roles):**

| Role pair | Compatible? |
|---|---|
| Searcher / Searcher | ✓ unbounded |
| Searcher / Inserter | ✓ |
| Inserter / Inserter | ✗ |
| Anyone / Deleter | ✗ |

**Invariants:**
- `searcher_count ≥ 0`, `inserter_count ≤ 1`.
- `deleter > 0 ⇒ searchers == 0 && inserters == 0`.

**English algorithm (lightswitch + 2 rooms):**
```
state: mtx, searcher_count, no_searcher_sem(1), no_inserter_sem(1)

search():
  mtx.lock; ++searcher_count
  if searcher_count==1: no_searcher.acquire    # first searcher claims
  mtx.unlock
  # actual search
  mtx.lock; --searcher_count
  if searcher_count==0: no_searcher.release
  mtx.unlock

insert():                                       # role B: max 1 concurrent
  no_inserter.acquire
  # actual insert
  no_inserter.release

delete():                                       # role C: exclusive
  no_searcher.acquire                          # lock order!
  no_inserter.acquire
  # actual delete
  no_inserter.release; no_searcher.release    # LIFO
```

**No-starve variant:** add a turnstile every entrant gates through; deleter HOLDS across both acquires.

**Top mistakes:**
1. **Init semaphores to 0** instead of 1 → first searcher blocks forever at startup. Semaphore=1 means "available."
2. **Counter for inserter** — unnecessary. Inserter role caps at 1; the binary semaphore IS the count.
3. **`std::mutex` for `no_searcher`** — UB. First searcher acquires; LAST searcher (different thread!) releases. Must be `counting_semaphore<1>`.
4. **Two deleters acquire in opposite order** → circular-wait deadlock. Document a global lock order.
5. **`std::list::push_back` for storage** → TSan flags concurrent searcher iteration as a race (no release barrier). Use `std::vector<T>(MAX_SLOTS)` + `atomic<size_t>` with release/acquire on size.

---

## Page 10 — BRIDGE CROSSING

**Rules:** Bridge holds K cars, two directions (N/S). Same-direction OK; opposite forbidden.

**Invariants:**
- `0 ≤ on ≤ K`.
- `on > 0 ⇒` all on-bridge cars share one direction.

**State (4 things):** `on`, `dir ∈ {Free, North, South}`, `waitN`, `waitS`.

**Three-clause predicate (north entry; south is symmetric):**
```
on < K                                          # capacity
AND (dir == North || dir == Free)               # direction match
AND (on == 0 || waitS == 0)                    # anti-barge: don't barge in
                                                # while opposite is queued
```

**Direction handoff at exit (load-bearing):**
```
on--
if on == 0:
  switch:
    dir==North && waitS>0:  dir = South        # opposite has priority
    dir==South && waitN>0:  dir = North
    waitN > 0:              dir = North        # else continue same-side
    waitS > 0:              dir = South
    default:                dir = Free
  cv.Broadcast()
```

**Code skeleton (Go):**
```go
func (b *Bridge) Enter(north bool) {
    b.mu.Lock(); defer b.mu.Unlock()
    var myDir int
    var myWait, oppWait *int
    if north { myDir=DirNorth; myWait=&b.waitN; oppWait=&b.waitS }
    else     { myDir=DirSouth; myWait=&b.waitS; oppWait=&b.waitN }
    *myWait++
    for !(b.on<K && (b.dir==myDir || b.dir==DirFree) && (b.on==0 || *oppWait==0)) {
        b.cv.Wait()
    }
    *myWait--
    if b.dir == DirFree { b.dir = myDir }
    b.on++
}
```

**Top mistakes:**
1. **Naive lightswitch (no anti-barge gate)** → starvation under steady same-direction load.
2. **Drop the `on == 0` carve-out** in predicate → both-sides-queued deadlock at drain (each refuses on "opposite is waiting").
3. **Reset `dir = DirFree` unconditionally on drain** without handoff → race resolves randomly; starvation possible.
4. **`Signal` instead of `Broadcast`** on drain → only one waiter wakes.

---

## Page 11 — LANGUAGE QUICK-REFERENCE

### C++ — primitive cheat

| Need | Code |
|---|---|
| Mutex | `std::mutex mu; std::lock_guard lk{mu};` |
| Cond | `std::condition_variable cv; cv.wait(lk, []{return pred;});` |
| Counting sem | `std::counting_semaphore<MAX> s{init};` |
| Multi-mutex | `std::scoped_lock{mu1, mu2};` |
| RW | `std::shared_mutex mu; std::shared_lock lk{mu};` |
| Sleep | `std::this_thread::sleep_for(1ms);` |

**Top C++ gotchas:** `lock(); ... unlock();` without RAII = leak risk. `notify` has no memory — set predicate UNDER mutex. `counting_semaphore` no FIFO. `release(N)` for shutdown broadcast.

### Rust sync — primitive cheat

| Need | Code |
|---|---|
| Mutex | `Arc<Mutex<T>>; let mut g = m.lock().unwrap();` |
| Cond | `let g = cv.wait_while(g, \|s\| pred(s)).unwrap();` |
| Atomic | `let new = a.fetch_add(1, Ordering::SeqCst) + 1;` |
| mpsc | `let (tx,rx) = mpsc::sync_channel(N);` |
| Sleep | `thread::sleep(Duration::from_millis(1));` |

**Top Rust sync gotchas:**
1. `let _ = m.lock();` — drops guard immediately. Use `let _g = m.lock().unwrap();` (named).
2. `wait_while` polarity: waits WHILE pred true (opposite of C++).
3. `wait_while` CONSUMES guard — shadow: `let g = m.lock(); let mut g = cv.wait_while(g, ...).unwrap();`
4. `True`/`False` are not Rust — use `true`/`false`.
5. Operators don't auto-deref through guards: `*g += 1`, not `g += 1`.

### Rust tokio — primitive cheat

| Need | Code |
|---|---|
| Mutex (across .await) | `Arc<tokio::sync::Mutex<T>>; let mut g = m.lock().await;` |
| Sem (pool, RAII) | `let _p = sem.acquire().await.unwrap();` |
| Sem (gate) | `sem.acquire().await.unwrap().forget();` + `add_permits(N)` to open |
| mpsc | `let (tx,mut rx) = mpsc::channel(N);` |
| oneshot | `let (tx,rx) = oneshot::channel();` |
| broadcast | `let (tx,_) = broadcast::channel(N); let mut rx = tx.subscribe();` |
| Sleep | `tokio::time::sleep(Duration::from_millis(1)).await;` |

**Top Rust tokio gotchas:**
1. **`std::sync::Mutex` across `.await` → future `!Send`.** Block-scope the guard, OR use `tokio::sync::Mutex`. `drop(g)` does NOT help (lexical analysis).
2. **`.acquire().await` returns `Result`** — `.unwrap()` after `.await`.
3. **Pool vs gate dichotomy**: pool = drop-permit (RAII); gate = `.forget()` + `add_permits()`.
4. **`drop(tx)` to close mpsc.** Main must drop its tx clone or consumer hangs.
5. **`broadcast::Lagged(n)`** is recoverable — `continue`, never `break`.
6. **Receiver isn't Clone** for mpsc/broadcast. mpsc → `Arc<Mutex<Receiver>>`; broadcast → `tx.subscribe()`. **Watch is the only one with Clone Receiver.**
7. **Clone Arc/sender OUTSIDE `async move`** in spawn loops, or value gets moved on iteration 0.

### Go — primitive cheat

| Need | Code |
|---|---|
| Mutex | `var mu sync.Mutex; mu.Lock(); defer mu.Unlock()` |
| Cond | `cv := sync.NewCond(&mu); for !pred { cv.Wait() }` |
| Channel (buffered) | `ch := make(chan T, N); ch <- x; x := <-ch` |
| Select | `select { case x:=<-c1: ... case c2<-y: ... case <-time.After(d): ... }` |
| Try (non-block) | `select { case ch<-x: ... default: ... }` |
| RWMutex | `var mu sync.RWMutex; mu.RLock(); ... mu.RUnlock();` |

**Top Go gotchas:**
1. `chan struct{}` cap 1 is a binary semaphore; cap 0 is rendezvous.
2. ONLY senders may close. Closing twice panics.
3. `select` without `default:` blocks; with `default:` is non-blocking.
4. `select` is **pseudo-random** between ready cases — not source order.
5. `for { select { ... } }` worker: don't `return` in work-arms; that exits the function.
6. Closer-goroutine pattern for multi-producer: `go func() { wg.Wait(); close(ch) }()`.
7. `close(done)` for broadcast cancellation — every blocked `<-done` returns.
8. **`return` vs `break` in select**: `break` exits the case (ineffective for the for-loop); use labeled break or `return`.

---

## Page 12 — DECISION TABLES + UNIVERSAL GOTCHAS

### `notify_one` vs `notify_all` — pick by "how many can pass?"

| Scenario | Use |
|---|---|
| PC: push 1 item (only 1 consumer can pass `!empty`) | `notify_one` |
| PC: pop frees 1 slot (only 1 producer can pass `!full`) | `notify_one` |
| Sushi drain ends — up to 5 can pass `!draining` | `notify_all` |
| Barrier threshold met — N-1 can proceed | `notify_all` |
| RW writer finishes — many readers can proceed | `notify_all` |

**When unsure: `notify_all`.** Wrong way is a silent lost-wakeup bug; right way with `notify_all` is just suboptimal, never incorrect.

### Mutex vs binary semaphore (decision rule)

| | Use mutex | Use binary semaphore |
|---|---|---|
| Same thread acquires + releases | ✓ | OK |
| Different threads (first/last reader; signaller/waiter) | **NO** (UB in C++) | ✓ |

**Rule:** acquire/release in same thread = mutex. Cross-thread = semaphore (permit semantics, no ownership).

### Counter-or-no-counter (lightswitch)

| Role's max concurrent | Need counter? |
|---|---|
| 1 (e.g., inserter, deleter, writer) | **NO** — semaphore IS the count |
| Unbounded (e.g., reader, searcher) | **YES** — first-in/last-out needs explicit count |

### Blocking semantic — pick by spec keyword

| Spec says | Use |
|---|---|
| (default) | block (`lock()`, `acquire()`, `send().await`) |
| "tries", "non-blocking" | `try_lock`, `try_acquire`, `try_send` |
| "with timeout", "after Xms" | `try_lock_for`, `tokio::time::timeout` |
| "leaves", "balks", "returns error", "queue full" | balk: `try_send` + `Err(Full)` → return |
| "drops oldest", "may miss" | lossy: broadcast `Lagged(n)`, watch overwrite |

### Shutdown patterns

| Primitive | Shutdown |
|---|---|
| Channel (Rust/Go) | drop last sender; receiver sees `None`/closed |
| Go cancellation | `close(done)` — every `<-done` returns |
| C++ semaphore | `release(MAX_WAITERS)` per side; self-heal on bail |
| C++ condvar | `notify_all()` + flag check |
| tokio mpsc | drop last sender (or sentinel msg) |

### Universal lessons (one-liner each)

| # | Lesson |
|---|---|
| A | Oracle (invariant tracker) state independent of algorithm state |
| B | Per-op atomicity ≠ multi-op atomicity. Compound ops need a mutex |
| C | Asymmetric bail: push bails on closed; pop drains then bails |
| D | Predicate polarity differs: C++ `wait(lk, pred)` until-true; Rust `wait_while` while-true |
| E | Where shutdown complexity lives differs by primitive — cv cheap, sem verbose |
| F | Liveness depends on primitive's wakeup fairness — std::counting_semaphore not FIFO |
| G | TSan structural; can't reason quantitatively (e.g., footman pigeonhole) |
| H | Sabotage experiments: predict → break → verify → revert |
| I | Counter only for unbounded role; bounded-1 role = semaphore is its own count |
| J | Mutex vs binary semaphore by acquire/release thread (cross-thread → semaphore) |
| K | Concurrency rules ARE consistency contracts (each compatible pair → weaker contract) |
| L | Don't add a flag if existing primitive state already encodes it |

### Quick failure-mode dictionary

| If you see... | Suspect... |
|---|---|
| Test passes once but flakes under load | predicate `if` instead of `while`; lost wakeup |
| TSan flags structural deadlock | lock-order inversion; missing lock around compound op |
| One thread eternally waiting | starvation (need turnstile); lost wakeup; closed predicate disjunct missing |
| Items lost / counts mismatch | symmetric bail; drop while waiters parked |
| Deadlock at startup | semaphore init wrong (0 vs 1); first-acquirer can't proceed |
| Future not Send (Rust tokio) | `std::sync::MutexGuard` across `.await` |
| "Use of moved value" in spawn loop | clone-into-`async move` instead of clone-then-move |
| Throughput collapses with M workers | lock held across the work (Q19 lock-scope bug) |
