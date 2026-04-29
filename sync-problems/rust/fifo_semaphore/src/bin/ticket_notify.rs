// FIFO Semaphore — tokio template (Rust)
//
// Scenario (lecture T7.2): build a counting semaphore that wakes waiters in
// arrival order. tokio::sync::Semaphore is roughly FIFO in practice but the
// lecture asks you to build the primitive yourself.
//
// Run:
//   cargo run -p fifo_semaphore --release
//
// Strategies (lecture demos 4-7, Task 1):
//   1. Ticket queue with atomic next_ticket + atomic now_serving + spinwait.
//      Each acquirer fetch_add()s next_ticket; release() fetch_add()s now_serving.
//      Spin while now_serving < my_ticket. In async land, spin with
//      tokio::task::yield_now().await so you don't starve the executor.
//   2. Same ticket queue, but instead of yielding, park on a tokio::sync::Notify
//      and have release() notify_one(). Naive Notify can wake the wrong waiter;
//      the safest fix is one Notify per ticket (or a per-waiter oneshot).
//   3. Queue of per-waiter oneshot::Sender — release() pops the front of a
//      Mutex<VecDeque<oneshot::Sender>> and sends to that one specifically.
//      No spinning, true FIFO by construction. Mirrors C++ FIFOSemaphore5.
//
// Test approach: spawn N tasks that each sleep i * SPACING_MS (so they call
// acquire() in spawn order), then verify wake_indices[i] == i after a chain
// of releases drains the queue.
//
// Caveat: the test depends on SPACING being large enough that earlier tasks
// reach acquire() before later ones. If you see flaky FIFO violations on a
// correct impl, bump ARRIVAL_SPACING_MS or reduce N_THREADS.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;

const N_THREADS: usize = 16;
const INITIAL_COUNT: usize = 0;
const ARRIVAL_SPACING_MS: u64 = 10;
const SLOTS: usize = N_THREADS + INITIAL_COUNT + 1;

// ==========================================================================
// TODO: Implement FifoSemaphore.
//
// API contract:
//   new(initial_count)
//   acquire().await: block until count > 0, then count -= 1.
//                    Waiters MUST wake in arrival order.
//   release():       count += 1; if a waiter is queued, the OLDEST one wakes.
// ==========================================================================
pub struct FifoSemaphore {
    now_serving: AtomicUsize,
    next_ticket: AtomicUsize,
    notify_list: [Notify; SLOTS],
}

impl FifoSemaphore {
    pub fn new(initial_count: usize) -> Self {
        let s = Self {
            now_serving: AtomicUsize::new(initial_count),
            next_ticket: AtomicUsize::new(0),
            notify_list: std::array::from_fn(|_| Notify::new()),
        };
        for i in 0..initial_count {
            s.notify_list[i].notify_one();
        }
        s
    }

    pub async fn acquire(&self) {
        let current_ticket = self.next_ticket.fetch_add(1, Ordering::SeqCst);
        self.notify_list[current_ticket].notified().await;
    }

    pub fn release(&self) {
        let finished_ticket = self.now_serving.fetch_add(1, Ordering::SeqCst);
        self.notify_list[finished_ticket].notify_one();
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
