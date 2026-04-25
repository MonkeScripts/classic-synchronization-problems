// Barrier — tokio + Semaphore variant (preloaded turnstile, slide 22).
//
// Sibling to ../main.rs (mutex + Condvar + generation counter).
// Rust's stdlib has no semaphore, so this version pulls in
// `tokio::sync::Semaphore` and runs the workers on the tokio runtime
// as async tasks.
//
// Run:
//     cargo run -p barrier --bin tokio --release
//
// ThreadSanitizer (nightly):
//     RUSTFLAGS="-Z sanitizer=thread" \
//         cargo +nightly run -p barrier --bin tokio --release
//
// ----------------------------------------------------------------------
// Why preloaded turnstile is the natural shape with a real semaphore:
//
//   Phase 1 (gather):
//     lock state; count += 1;
//     if count == expected: t1.add_permits(expected);
//     unlock state;
//     t1.acquire().await — and FORGET the permit so it's consumed.
//
//   Phase 2 (disperse):
//     lock state; count -= 1;
//     if count == 0: t2.add_permits(expected);
//     unlock state;
//     t2.acquire().await — and FORGET.
//
// The `forget()` is the trick: a normal `SemaphorePermit` returns its
// permit to the semaphore on drop. We want the permit *consumed* (each
// thread takes one and it's gone), which is what `forget()` does.
// Without it, `add_permits(N)` followed by N acquires plus N drops just
// puts all permits back — the semaphore never empties, the gate never
// closes, and round 2 lets everyone through prematurely.
//
// `std::sync::Mutex` is fine here (sync) because the critical section
// doesn't `.await` — tokio's docs explicitly recommend std Mutex over
// `tokio::sync::Mutex` when you don't hold the lock across awaits.
// ----------------------------------------------------------------------

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::Semaphore;

const N_THREADS: usize = 8;
const ROUNDS: usize = 100;

// ==========================================================================
// TODO: Replace this stub with YOUR implementation following the pattern
// in the header comment.
// ==========================================================================
pub struct MyBarrierTokio {
    expected: usize,
    state: Mutex<BarrierState>,
    t1: Semaphore,
    t2: Semaphore,
}

struct BarrierState {
    count: usize,
}

impl MyBarrierTokio {
    pub fn new(expected: usize) -> Self {
        Self {
            expected,
            state: Mutex::new(BarrierState { count: 0 }),
            t1: Semaphore::new(0),
            t2: Semaphore::new(0),
        }
    }

    pub async fn arrive_and_wait(&self) {
        // TODO: implement preloaded two-turnstile.
        //
        // Phase 1:
        //   {
        //       let mut state = self.state.lock().unwrap();
        //       state.count += 1;
        //       if state.count == self.expected {
        //           self.t1.add_permits(self.expected);
        //       }
        //   } // drop the std::sync::MutexGuard BEFORE awaiting
        //   self.t1.acquire().await.unwrap().forget();
        //
        // Phase 2: mirror image with t2 and count == 0.
        //
        // Pitfalls:
        //   (a) Don't hold the std Mutex guard across `.await` — that's
        //       a deadlock waiting to happen and will get flagged.
        //   (b) `acquire()` returns Result<SemaphorePermit, AcquireError>.
        //       AcquireError only happens if you `close()` the semaphore;
        //       we never do, so `.unwrap()` is fine.
        //   (c) Forgetting `.forget()` is the canonical bug — see header.
        {
            let mut state = self.state.lock().unwrap();
            state.count += 1;
            if state.count == self.expected {
                self.t1.add_permits(self.expected);
            }
        }
        self.t1.acquire().await.unwrap().forget();

        {
            let mut state = self.state.lock().unwrap();
            state.count -= 1;
            if state.count == 0 {
                self.t2.add_permits(self.expected);
            }
        }
        self.t2.acquire().await.unwrap().forget();
    }
}

// --------------------------------------------------------------------------
// Invariant tracking — same shape as main.rs.
// --------------------------------------------------------------------------
static GLOBAL_ROUND: AtomicUsize = AtomicUsize::new(0);
static ARRIVED_THIS_ROUND: AtomicUsize = AtomicUsize::new(0);

async fn worker(tid: usize, barrier: Arc<MyBarrierTokio>) {
    for r in 0..ROUNDS {
        // pretend work
        let mut x: u64 = 0;
        for i in 0..1000u64 {
            x = x.wrapping_add(i);
        }
        std::hint::black_box(x);

        barrier.arrive_and_wait().await;

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

        barrier.arrive_and_wait().await;
    }
}

#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() {
    let barrier = Arc::new(MyBarrierTokio::new(N_THREADS));
    let start = Instant::now();
    let handles: Vec<_> = (0..N_THREADS)
        .map(|t| {
            let b = Arc::clone(&barrier);
            tokio::spawn(async move { worker(t, b).await })
        })
        .collect();
    for h in handles {
        h.await.unwrap();
    }
    println!(
        "Completed {} rounds with {} threads in {} ms",
        ROUNDS,
        N_THREADS,
        start.elapsed().as_millis()
    );
}
