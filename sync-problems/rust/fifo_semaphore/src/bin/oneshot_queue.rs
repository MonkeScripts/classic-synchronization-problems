// FIFO Semaphore — oneshot-queue strategy (Strategy 3) — tokio template
//
// Scenario (lecture T7.2): build a counting semaphore that wakes waiters in
// arrival order. tokio::sync::Semaphore is roughly FIFO in practice but the
// lecture asks you to build the primitive yourself.
//
// Run:
//   cargo run -p fifo_semaphore --bin oneshot_queue --release
//
// Compare against:
//   cargo run -p fifo_semaphore --bin ticket_notify --release
//
// ---------------------------------------------------------------------------
// Strategy: queue of per-waiter oneshot::Sender. Mirrors C++ FIFOSemaphore5.
// ---------------------------------------------------------------------------
//
//   State guarded by ONE std::sync::Mutex (NOT tokio::sync::Mutex — see below):
//     struct State {
//         count: usize,
//         waiters: VecDeque<oneshot::Sender<()>>,
//     }
//
//   acquire():
//     lock state
//     if count > 0:
//         count -= 1
//         drop lock; return
//     else:
//         let (tx, rx) = oneshot::channel()
//         waiters.push_back(tx)
//         drop lock
//         rx.await
//
//   release():
//     lock state
//     if let Some(tx) = waiters.pop_front():
//         drop lock; let _ = tx.send(())   // permit transfers DIRECTLY,
//                                          // count stays the same
//     else:
//         count += 1
//
// FIFO is automatic: VecDeque preserves push order, pop_front always wakes
// the OLDEST waiter, and the oneshot send/recv is unambiguous (no shared
// Notify slots, no permit-can-wake-the-wrong-task hazard).
//
// ---------------------------------------------------------------------------
// Why std::sync::Mutex (not tokio::sync::Mutex):
//   The hot path is push_back + drop. We never .await while holding the lock,
//   so std Mutex is faster and doesn't infect callers with async. tokio's
//   own docs recommend std Mutex for short non-awaiting critical sections.
//
// Why a mutex IS needed here (unlike ticket_notify.rs):
//   State is COMPOUND. release() must observe `pop_front` AND `send` as one
//   step; acquire()'s "check count, decrement, push" is also compound. A
//   single fetch_add can't express that. See fifo-semaphore-learnings.md
//   Question 4e.
//
// CRITICAL: do not hold the std::sync::Mutex guard across .await. Build the
// channel and push tx INSIDE a {} block so the guard drops, THEN await:
//
//     let rx = {
//         let mut s = self.state.lock().unwrap();
//         ...
//         rx
//     };
//     rx.await.unwrap();
//
// ---------------------------------------------------------------------------
// Test approach: identical harness to ticket_notify.rs — N tasks call
// acquire() in spawn order (each sleeps i*SPACING_MS first), and we verify
// wake_indices[i] == i after a chain of releases drains the queue.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;

const N_THREADS: usize = 16;
const INITIAL_COUNT: usize = 0;
const ARRIVAL_SPACING_MS: u64 = 10;

// ==========================================================================
// TODO: Implement FifoSemaphore.
//
// API contract:
//   new(initial_count)
//   acquire().await: block until count > 0, then count -= 1.
//                    Waiters MUST wake in arrival order.
//   release():       count += 1; if a waiter is queued, the OLDEST one wakes.
//                    (In practice: hand the permit DIRECTLY to the front
//                     waiter without bumping count.)
// ==========================================================================

struct State {
    count: usize,
    waiters: VecDeque<oneshot::Sender<()>>,
}

pub struct FifoSemaphore {
    state: Mutex<State>,
}

impl FifoSemaphore {
    pub fn new(initial_count: usize) -> Self {
        // TODO: initialize state with count = initial_count and an empty
        // waiters queue. No pre-deposit dance is needed here — count > 0
        // is the fast path; there are no Notify slots to seed.
        Self {
            state: Mutex::new(State {
                count: initial_count,
                waiters: VecDeque::new(),
            }),
        }
    }

    pub async fn acquire(&self) {
        let rx = {
            let mut state = self.state.lock().unwrap();
            if state.count > 0 {
                state.count -= 1;
                return;
            }
            let (tx, rx) = oneshot::channel();
            state.waiters.push_back(tx);
            rx
        };
        rx.await.unwrap();
    }

    pub fn release(&self) {
        let mut state = self.state.lock().unwrap();
        if let Some(tx) = state.waiters.pop_front() {
            drop(state);
            let _ = tx.send(());
        } else {
            state.count += 1;
        }
    }
}

// ----- Invariant tracking (do not touch) -----
static GLOBAL_WAKE: AtomicUsize = AtomicUsize::new(0);

#[tokio::main]
async fn main() {
    let sem = Arc::new(FifoSemaphore::new(INITIAL_COUNT));
    let wake_indices: Arc<Vec<AtomicUsize>> =
        Arc::new((0..N_THREADS).map(|_| AtomicUsize::new(usize::MAX)).collect());

    let mut handles = Vec::new();
    let start = std::time::Instant::now();

    for i in 0..N_THREADS {
        let sem = Arc::clone(&sem);
        let wake_indices = Arc::clone(&wake_indices);
        handles.push(tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(i as u64 * ARRIVAL_SPACING_MS)).await;
            sem.acquire().await;
            let wake = GLOBAL_WAKE.fetch_add(1, Ordering::SeqCst);
            wake_indices[i].store(wake, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(2)).await;
            sem.release();
        }));
    }

    // Give every task time to call acquire() and queue up.
    tokio::time::sleep(Duration::from_millis(((N_THREADS as u64) + 2) * ARRIVAL_SPACING_MS)).await;
    sem.release(); // kick off the chain

    for h in handles {
        h.await.unwrap();
    }
    let elapsed = start.elapsed();

    let mut violations = 0;
    for i in 0..N_THREADS {
        let w = wake_indices[i].load(Ordering::SeqCst);
        if w != i {
            eprintln!("FIFO violation: thread {} woke at index {}", i, w);
            violations += 1;
        }
    }
    println!("Took {:?}, {} FIFO violations", elapsed, violations);
    if violations > 0 {
        std::process::exit(1);
    }
}
