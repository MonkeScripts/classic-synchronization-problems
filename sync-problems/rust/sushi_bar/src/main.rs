// Sushi Bar — Downey 5.1 — TEMPLATE (Rust sync, std::sync::{Mutex, Condvar})
//
// 5 seats. Customers arrive, sit, eat, leave. Once 5 are SIMULTANEOUSLY seated,
// the gate closes (draining mode) and new arrivals park on a Condvar until the
// entire 5-group has fully drained. After drain, gate reopens; cycle repeats.
//
// If 5 are never simultaneously seated, drain mode never triggers — that's a
// legal run, all customers served, gate stays open the whole time.
//
// Run:
//   cargo run -p sushi_bar --release
//
// =============================================================================
// SUGGESTED DESIGN (mutex + cv + state struct — see STUDENT_WALKTHROUGH_SUSHI_BAR.md §0.2):
//
//   struct State {
//       seated: usize,         // 0..=5
//       must_leave: usize,     // 0..=5; how many of locked group still owe a leave
//       draining: bool,        // gate closed?
//   }
//
//   one Condvar over the state Mutex.
//
//   arrive(id):
//     1. Lock state.
//     2. cv.wait_while(state, |s| s.draining)               // park while draining
//     3. state.seated += 1
//     4. if state.seated == GROUP_SIZE:
//            state.draining = true; state.must_leave = GROUP_SIZE
//     (drop guard)                                           // release BEFORE eating
//
//     6. eat(id);                                            // outside lock
//
//     7. Re-lock state.
//     8. if state.draining:
//            state.must_leave -= 1; state.seated -= 1
//            if state.must_leave == 0:
//                state.draining = false; cv.notify_all()
//        else:
//            state.seated -= 1
//     (drop guard implicitly at function end)
//
// Why each piece:
//   - Predicate: `|s| s.draining` waits WHILE draining is true (Rust polarity:
//     wait_while waits while pred is TRUE; opposite of C++ — universal lesson D).
//   - Threshold flip on the 5th seater: only this customer flips draining true.
//   - Drain detection by must_leave (counter for the locked group): the LAST of
//     the 5 to leave flips draining false and broadcasts.
//   - notify_all (not notify_one) on gate reopen: many waiters may now pass the
//     predicate; up to GROUP_SIZE of them will fit before the gate closes again.
//
// Load-bearing details:
//   - The seated++ / threshold check / draining flip MUST all be under the SAME
//     lock acquisition — otherwise compound-op race (universal lesson B). Two
//     customers could both observe seated==4 and both bump to 5.
//   - The must_leave-- / draining flip / notify_all MUST all be under the SAME
//     lock acquisition — otherwise lost wakeup; a parked arrival between the
//     predicate check and cv.Wait() would miss the broadcast.
//   - eat() runs OUTSIDE the lock — otherwise only one customer eats at a time.
//   - The post-eat path's `if state.draining` check matters: customers who sat
//     during open-mode (not part of a locked group) decrement seated but DO NOT
//     touch must_leave or draining.
//
// Invariants enforced by the harness:
//   - SEATED_NOW <= GROUP_SIZE always (asserts in eat()). If a customer entered
//     while a previous group was draining, this fires.
//   - SERVED == N_CUSTOMERS at the end.

use rand::Rng;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

const N_CUSTOMERS: usize = 30;
const GROUP_SIZE: usize = 5;
const MEAN_INTER_ARRIVAL_MS: u64 = 5;
const MEAN_EAT_MS: u64 = 20;

// ----- Invariant tracking (do not touch) -----
static SEATED_NOW: AtomicUsize = AtomicUsize::new(0);
static MAX_SEATED: AtomicUsize = AtomicUsize::new(0);
static SERVED: AtomicUsize = AtomicUsize::new(0);

fn update_max(now: usize) {
    let mut cur = MAX_SEATED.load(Ordering::SeqCst);
    while now > cur {
        match MAX_SEATED.compare_exchange(cur, now, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => break,
            Err(actual) => cur = actual,
        }
    }
}

/// The "actual work" — call this BETWEEN the two locked regions of arrive().
/// DO NOT modify.
fn eat(_id: usize) {
    let now = SEATED_NOW.fetch_add(1, Ordering::SeqCst) + 1;
    assert!(
        now <= GROUP_SIZE,
        "INVARIANT: seated_now={} > GROUP_SIZE={} — a customer entered while \
         a previous group was still draining",
        now,
        GROUP_SIZE
    );
    update_max(now);

    let ms = {
        let mut rng = rand::thread_rng();
        rng.gen_range(MEAN_EAT_MS / 2..=MEAN_EAT_MS * 2)
    };
    thread::sleep(Duration::from_millis(ms));

    SEATED_NOW.fetch_sub(1, Ordering::SeqCst);
    SERVED.fetch_add(1, Ordering::SeqCst);
}

// =============================================================================
// State struct guarded by the Mutex.
// =============================================================================
struct State {
    seated: usize,
    must_leave: usize,
    draining: bool,
}

pub struct SushiBar {
    state: Mutex<State>,
    cv: Condvar,
}

impl SushiBar {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State {
                seated: 0,
                must_leave: 0,
                draining: false,
            }),
            cv: Condvar::new(),
        }
    }

    pub fn arrive(&self, _id: usize) {
        {
            let mut shared = self.state.lock().unwrap();
            shared = self.cv.wait_while(shared, |s| {
                s.draining
            }).unwrap();
            shared.seated += 1
            if shared.seated == GROUP_SIZE {
                shared.must_leave = GROUP_SIZE;
                shared.draining = true;
            }

        }
        eat(_id);
        {
            let mut shared = self.state.lock().unwrap();
            if shared.draining {
                shared.seated -= 1;
                shared.must_leave -= 1;
                if shared.must_leave == 0 {
                    shared.draining = false;
                    self.cv.notify_all();
                }
            }
            else {
                shared.seated -= 1;
            }
        }
        
    }
}

fn main() {
    let bar = Arc::new(SushiBar::new());
    let start = std::time::Instant::now();

    let mut handles = Vec::new();
    for id in 0..N_CUSTOMERS {
        let bar = Arc::clone(&bar);

        // Stagger arrivals so the bar actually fills up.
        let delay = {
            let mut rng = rand::thread_rng();
            rng.gen_range(0..=MEAN_INTER_ARRIVAL_MS * 2)
        };
        thread::sleep(Duration::from_millis(delay));

        handles.push(thread::spawn(move || {
            bar.arrive(id);
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    let elapsed = start.elapsed();

    let served = SERVED.load(Ordering::SeqCst);
    let max_seated = MAX_SEATED.load(Ordering::SeqCst);

    println!(
        "customers: {}  group_size: {}",
        N_CUSTOMERS, GROUP_SIZE
    );
    println!(
        "served: {} / {}  max_seated: {} (cap: {})",
        served, N_CUSTOMERS, max_seated, GROUP_SIZE
    );
    println!("time: {:?}", elapsed);

    let mut ok = true;
    if served != N_CUSTOMERS {
        eprintln!(
            "INVARIANT: served {} != N_CUSTOMERS {}",
            served, N_CUSTOMERS
        );
        ok = false;
    }
    if max_seated > GROUP_SIZE {
        eprintln!(
            "INVARIANT: max_seated {} > GROUP_SIZE {}",
            max_seated, GROUP_SIZE
        );
        ok = false;
    }
    if max_seated < 2 {
        eprintln!(
            "WARNING: max_seated only {} — load may be too light to exercise drain mode",
            max_seated
        );
        // not a hard fail; a run where 5 never overlap is legal
    }
    if !ok {
        std::process::exit(1);
    }
}
