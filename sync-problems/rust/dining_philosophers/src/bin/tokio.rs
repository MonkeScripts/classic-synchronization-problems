// Dining Philosophers — tokio (async) starter template
//
// Five strategies, mirroring the C++ + Go work combined:
//   1. eat_naive       — left then right. Deadlocks on purpose. Demo only.
//   2. eat_asymmetric  — last philosopher reverses. Breaks the cycle structurally.
//   3. eat_try_backoff — tokio Mutex try_lock + retry. Channel of std::scoped_lock.
//   4. eat_footman     — tokio Semaphore caps N-1 hungry philosophers.
//   5. eat_tanenbaum   — per-philosopher state + Notify. No chopstick locks.
//
// Run all five sequentially:
//     cargo run -p dining_philosophers --bin tokio --release
//
// Each strategy is a TODO block — fill in the bodies. Shared state + the
// run_strategy harness are already wired.

use rand::Rng;
use std::sync::atomic::{AtomicI32, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Notify, Semaphore};

const N: usize = 5;
const MEALS_PER_PHILOSOPHER: i32 = 50;

// State as u8 so we can put it in AtomicU8 (oracle) and a plain [u8; N] (algo).
const THINKING: u8 = 0;
const HUNGRY: u8 = 1;
const EATING: u8 = 2;

// =============================================================================
// Shared state — every strategy gets a fresh `Arc<SharedTokio>` per run.
// =============================================================================

struct SharedTokio {
    // ---- Oracle (witness; never algorithm state) ----
    states: [AtomicU8; N],
    meals: [AtomicI32; N],

    // ---- Resources used by the chopstick-lock strategies ----
    chopsticks: [Mutex<()>; N],

    // ---- Resources used by eat_footman ----
    num_eaters: Semaphore,

    // ---- Resources used by eat_tanenbaum ----
    // algo_states is the algorithm's OWN state — separate from `states`
    // (the oracle), per the readers-writers / dining-philosophers C++ writeups.
    algo_states: Mutex<[u8; N]>,
    notify: [Notify; N],
}

impl SharedTokio {
    fn new() -> Self {
        Self {
            states: std::array::from_fn(|_| AtomicU8::new(THINKING)),
            meals: std::array::from_fn(|_| AtomicI32::new(0)),
            chopsticks: std::array::from_fn(|_| Mutex::new(())),
            num_eaters: Semaphore::new(N - 1),
            algo_states: Mutex::new([THINKING; N]),
            notify: std::array::from_fn(|_| Notify::new()),
        }
    }

    fn check_invariant(&self, pid: usize) {
        let left = (pid + N - 1) % N;
        let right = (pid + 1) % N;
        assert_ne!(
            self.states[left].load(Ordering::SeqCst),
            EATING,
            "philosopher {pid} eating while left {left} is eating"
        );
        assert_ne!(
            self.states[right].load(Ordering::SeqCst),
            EATING,
            "philosopher {pid} eating while right {right} is eating"
        );
    }

    /// Tanenbaum helper. PRECONDITION: caller holds `algo_states` lock and
    /// passes the guard's `&mut [u8; N]` in `state`.
    fn tanenbaum_test(&self, state: &mut [u8; N], pid: usize) {
        let left = (pid + N - 1) % N;
        let right = (pid + 1) % N;
        if state[pid] == HUNGRY && state[left] != EATING && state[right] != EATING {
            state[pid] = EATING;
            // Notify HAS the "deposited permit" property when no one is
            // waiting — the next .notified().await consumes it without
            // blocking. Same memory property the semaphore-Tanenbaum
            // walkthrough described.
            self.notify[pid].notify_one();
        }
    }
}

// =============================================================================
// Helpers
// =============================================================================

async fn jitter(max_ms: u64) {
    let ms = rand::thread_rng().gen_range(0..=max_ms);
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

async fn think(shared: &SharedTokio, pid: usize) {
    shared.states[pid].store(THINKING, Ordering::SeqCst);
    jitter(3).await;
}

// =============================================================================
// Strategy 1: Naive (left then right) — DEADLOCKS. Demo only.
// =============================================================================
// Every philosopher acquires left first, then right. With N=5 and any
// contention, this forms the cycle 0→1→2→3→4→0 and hangs. tokio's runtime
// won't print "all tasks asleep" the way Go's does — it just hangs silently.
// Wrap the call in `tokio::time::timeout` if you want to demonstrate it.

async fn eat_naive(shared: &SharedTokio, pid: usize) {
    shared.states[pid].store(HUNGRY, Ordering::SeqCst);

    let _left = pid;
    let _right = (pid + 1) % N;

    // TODO: let _l = shared.chopsticks[_left].lock().await;
    // TODO: let _r = shared.chopsticks[_right].lock().await;

    shared.states[pid].store(EATING, Ordering::SeqCst);
    shared.check_invariant(pid);
    shared.meals[pid].fetch_add(1, Ordering::SeqCst);
    jitter(3).await;
    shared.states[pid].store(THINKING, Ordering::SeqCst);

    // No explicit release needed: _l and _r are tokio::sync::MutexGuards;
    // they release on Drop in REVERSE declaration order at scope end (LIFO).
}

// =============================================================================
// Strategy 2: Asymmetric (last philosopher reverses)
// =============================================================================
// Philosopher N-1 grabs right-first; everyone else grabs left-first. Breaks
// the cycle in the lock-acquire graph: P4 holds cs0 waiting cs4, but cs0 is
// the second-acquire for nobody else, so the cycle never closes.

async fn eat_asymmetric(shared: &SharedTokio, pid: usize) {
    shared.states[pid].store(HUNGRY, Ordering::SeqCst);

    // TODO: pick (first_idx, second_idx):
    //         if pid == N - 1: (right=(pid+1)%N, left=pid)   // reversed
    //         else:            (left=pid, right=(pid+1)%N)   // normal
    // TODO: let _f = shared.chopsticks[first_idx].lock().await;
    // TODO: let _s = shared.chopsticks[second_idx].lock().await;

    shared.states[pid].store(EATING, Ordering::SeqCst);
    shared.check_invariant(pid);
    shared.meals[pid].fetch_add(1, Ordering::SeqCst);
    jitter(3).await;
    shared.states[pid].store(THINKING, Ordering::SeqCst);

    // _s, _f drop in reverse — RAII releases.
}

// =============================================================================
// Strategy 3: try-and-back-off (tokio::sync::Mutex::try_lock)
// =============================================================================
// Block-acquire one, NON-BLOCKING try the other. If the try fails, drop the
// first and retry in the OTHER ORDER. Either both held when we exit the loop,
// or neither held at the bottom of the loop body. Same shape as the Go
// labeled-break version, but in Rust we use a regular `loop { ... return; }`
// because Rust doesn't need labels for nested control flow here.
//
// tokio::sync::Mutex::try_lock() returns Result<MutexGuard, TryLockError>:
//   - Ok(guard)  → got it
//   - Err(_)     → not available right now (someone else holds it)
//
// Liveness: technically livelock-prone in theoretical lockstep. tokio's
// fair-FIFO scheduling + jitter from think() prevents it in practice.

async fn eat_try_backoff(shared: &SharedTokio, pid: usize) {
    shared.states[pid].store(HUNGRY, Ordering::SeqCst);

    let _left = pid;
    let _right = (pid + 1) % N;

    // TODO: loop {
    //   // Phase A: lock left, try right
    //   let l_guard = shared.chopsticks[left].lock().await;
    //   match shared.chopsticks[right].try_lock() {
    //       Ok(r_guard) => {
    //           // hold both — do the eat (set EATING, check, count, jitter, THINKING)
    //           // then return; (drop r_guard, l_guard at end of fn — LIFO RAII)
    //       }
    //       Err(_) => { drop(l_guard); /* try other order */ }
    //   }
    //
    //   // Phase B: lock right, try left
    //   let r_guard = shared.chopsticks[right].lock().await;
    //   match shared.chopsticks[left].try_lock() {
    //       Ok(l_guard) => { /* eat, return */ }
    //       Err(_) => { drop(r_guard); /* loop again */ }
    //   }
    // }
    //
    // Note: the eat-section (states.store(EATING), check_invariant, meals,
    // jitter, states.store(THINKING)) appears inside BOTH match arms.
    // Easiest is to factor it into a helper or just duplicate.

    // Placeholder so the function compiles before TODOs are filled in:
    shared.states[pid].store(EATING, Ordering::SeqCst);
    shared.check_invariant(pid);
    shared.meals[pid].fetch_add(1, Ordering::SeqCst);
    jitter(3).await;
    shared.states[pid].store(THINKING, Ordering::SeqCst);
}

// =============================================================================
// Strategy 4: Footman (tokio Semaphore caps N-1 hungry philosophers)
// =============================================================================
// Pigeonhole: with at most N-1 contenders for N chopsticks, no full cycle
// can form. Sequential lock-left-then-right is safe under the cap.
//
// tokio Semaphore difference from C++ counting_semaphore:
//   - Permits are RAII (SemaphorePermit). Dropping releases. No .release().
//   - tokio Semaphore is FIFO-fair (per docs). The C++ counting_semaphore
//     wasn't — that's the readers-writers fairness gap from the recap.

async fn eat_footman(shared: &SharedTokio, pid: usize) {
    shared.states[pid].store(HUNGRY, Ordering::SeqCst);

    // TODO: let _ticket = shared.num_eaters.acquire().await.unwrap();
    //                     ^ holds the cap-permit until end of scope

    let _left = pid;
    let _right = (pid + 1) % N;

    // TODO: let _l = shared.chopsticks[_left].lock().await;
    // TODO: let _r = shared.chopsticks[_right].lock().await;

    shared.states[pid].store(EATING, Ordering::SeqCst);
    shared.check_invariant(pid);
    shared.meals[pid].fetch_add(1, Ordering::SeqCst);
    jitter(3).await;
    shared.states[pid].store(THINKING, Ordering::SeqCst);

    // _r, _l, _ticket drop in reverse: chopstick R, chopstick L, footman ticket.
    // (Same LIFO release order as C++ scoped_lock + RAII semaphore-permit.)
}

// =============================================================================
// Strategy 5: Tanenbaum (state-tracking, no chopstick locks)
// =============================================================================
// All algo_states reads + decisions + writes happen under one mutex. When a
// philosopher finishes, it test()s its neighbors; test() may transition a
// neighbor to EATING and notify. Because notify_one() deposits a permit when
// no one is waiting, the .notified().await line absorbs the difference
// between "I was already given the green light by my own test" (Phase A) and
// "I'm waiting for a neighbor to release" (Phase B) — same shape as the
// C++ semaphore-Tanenbaum walkthrough.
//
// Note: Notify is NOT a Condvar. It has no predicate-loop overload. We don't
// need one here because:
//   - Each tanenbaum_test under the lock is the ONLY way to set EATING.
//   - Once EATING is set for me, no one un-sets it (no race undoes it).
//   - Notify::notify_one() deposits at most one permit; .notified().await
//     consumes it deterministically. No spurious wakeups in tokio's impl.

async fn eat_tanenbaum(shared: &SharedTokio, pid: usize) {
    shared.states[pid].store(HUNGRY, Ordering::SeqCst);

    // ===== take_forks =====
    // TODO: {
    //   let mut state = shared.algo_states.lock().await;
    //   state[pid] = HUNGRY;
    //   shared.tanenbaum_test(&mut state, pid);
    // } // lock dropped here
    // TODO: shared.notify[pid].notified().await;
    //   // either consumes the permit deposited by test_self (Phase A),
    //   // or blocks until a neighbor's put_forks signals us (Phase B).

    shared.states[pid].store(EATING, Ordering::SeqCst);
    shared.check_invariant(pid);
    shared.meals[pid].fetch_add(1, Ordering::SeqCst);
    jitter(3).await;
    shared.states[pid].store(THINKING, Ordering::SeqCst);

    // ===== put_forks =====
    // TODO: {
    //   let mut state = shared.algo_states.lock().await;
    //   state[pid] = THINKING;
    //   let left  = (pid + N - 1) % N;
    //   let right = (pid + 1) % N;
    //   shared.tanenbaum_test(&mut state, left);
    //   shared.tanenbaum_test(&mut state, right);
    // }
}

// =============================================================================
// Harness
// =============================================================================

#[derive(Copy, Clone)]
enum Strategy {
    Naive,
    Asymmetric,
    TryBackoff,
    Footman,
    Tanenbaum,
}

async fn philosopher(shared: Arc<SharedTokio>, pid: usize, strat: Strategy) {
    for _ in 0..MEALS_PER_PHILOSOPHER {
        think(&shared, pid).await;
        match strat {
            Strategy::Naive => eat_naive(&shared, pid).await,
            Strategy::Asymmetric => eat_asymmetric(&shared, pid).await,
            Strategy::TryBackoff => eat_try_backoff(&shared, pid).await,
            Strategy::Footman => eat_footman(&shared, pid).await,
            Strategy::Tanenbaum => eat_tanenbaum(&shared, pid).await,
        }
    }
}

async fn run_strategy(name: &str, strat: Strategy) {
    let shared = Arc::new(SharedTokio::new());
    let start = Instant::now();

    let handles: Vec<_> = (0..N)
        .map(|pid| {
            let s = Arc::clone(&shared);
            tokio::spawn(async move { philosopher(s, pid, strat).await })
        })
        .collect();

    for h in handles {
        h.await.unwrap();
    }

    let elapsed = start.elapsed();
    println!("=== {} ===", name);
    println!("Done in {} ms", elapsed.as_millis());
    let meals: Vec<i32> = shared.meals.iter().map(|m| m.load(Ordering::SeqCst)).collect();
    println!("Meals per philosopher: {meals:?}");
    let min = *meals.iter().min().unwrap();
    let max = *meals.iter().max().unwrap();
    println!("min={min} max={max} spread={}\n", max - min);
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    // Naive will hang — leave it commented out unless you want to demonstrate
    // the deadlock. Use `tokio::time::timeout(Duration::from_secs(2), ...)`
    // to bound the demonstration.
    // run_strategy("naive", Strategy::Naive).await;

    run_strategy("asymmetric", Strategy::Asymmetric).await;
    run_strategy("try_backoff", Strategy::TryBackoff).await;
    run_strategy("footman", Strategy::Footman).await;
    run_strategy("tanenbaum", Strategy::Tanenbaum).await;
}
