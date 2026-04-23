// Barrier — starter template (Rust)
//
// Your job: implement `MyBarrier` with a reusable barrier. Try at least two:
//   1. Mutex<(count, generation)> + Condvar — classic.
//   2. Two turnstile semaphores (Rust stdlib doesn't have semaphores,
//      so build one from Mutex+Condvar, or pull in `tokio::sync::Semaphore`
//      / `crossbeam::sync::WaitGroup` / etc).
//   3. (Compare) std::sync::Barrier — the stdlib version.
//
// Invariant checked by the `round` counter + assert in worker().

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Instant;

const N_THREADS: usize = 8;
const ROUNDS: usize = 100;

// ==========================================================================
// TODO: Replace this placeholder with YOUR implementation.
// ==========================================================================
pub struct MyBarrier {
    expected: usize,
    state: Mutex<BarrierState>,
    cv: Condvar,
}

struct BarrierState {
    count: usize,
    generation: usize,
}

impl MyBarrier {
    pub fn new(expected: usize) -> Self {
        Self {
            expected,
            state: Mutex::new(BarrierState { count: 0, generation: 0 }),
            cv: Condvar::new(),
        }
    }

    pub fn arrive_and_wait(&self) {
        // TODO: implement. Hint:
        //   - lock state
        //   - remember `gen = state.generation`
        //   - state.count += 1
        //   - if state.count == expected:
        //         state.count = 0
        //         state.generation += 1
        //         notify_all
        //   - else: wait while state.generation == gen
        let _ = &self.state;
        let _ = &self.cv;
        let _ = self.expected;
    }
}

// Invariant tracking
static GLOBAL_ROUND: AtomicUsize = AtomicUsize::new(0);
static ARRIVED_THIS_ROUND: AtomicUsize = AtomicUsize::new(0);

fn worker(tid: usize, barrier: Arc<MyBarrier>) {
    for r in 0..ROUNDS {
        // pretend work
        let mut x: u64 = 0;
        for i in 0..1000u64 {
            x = x.wrapping_add(i);
        }
        std::hint::black_box(x);

        barrier.arrive_and_wait();

        let seen = GLOBAL_ROUND.load(Ordering::SeqCst);
        assert_eq!(
            seen, r,
            "thread {tid} saw round {seen} but expected {r}"
        );
        let arrived = ARRIVED_THIS_ROUND.fetch_add(1, Ordering::SeqCst) + 1;
        if arrived == N_THREADS {
            ARRIVED_THIS_ROUND.store(0, Ordering::SeqCst);
            GLOBAL_ROUND.store(r + 1, Ordering::SeqCst);
        }

        barrier.arrive_and_wait();
    }
}

fn main() {
    let barrier = Arc::new(MyBarrier::new(N_THREADS));
    let start = Instant::now();
    let handles: Vec<_> = (0..N_THREADS)
        .map(|t| {
            let b = Arc::clone(&barrier);
            thread::spawn(move || worker(t, b))
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    println!(
        "Completed {} rounds with {} threads in {} ms",
        ROUNDS,
        N_THREADS,
        start.elapsed().as_millis()
    );
}
