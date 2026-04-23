// Barbershop — starter template (Rust)
//
// Rust stdlib has no semaphore. Three realistic shapes for the solution:
//   1. Mutex<usize> (customer count) + Condvar for "barber available"
//      + Condvar for "customer ready" + Condvar for "done" — 4 condvars,
//      mirrors the semaphore-based textbook solution.
//   2. mpsc::sync_channel(n) as the waiting room (bounded),
//      + channels for barber/customer rendezvous. More idiomatic Rust.
//   3. tokio version (stretch — async/await).
//
// Pick ONE, get it working, then try the other.
//
// Invariants:
//   - served + balked == TOTAL_CUSTOMERS
//   - customer count never exceeds CHAIRS

use rand::Rng;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

const CHAIRS: usize = 3;
const TOTAL_CUSTOMERS: i32 = 30;

pub struct Barbershop {
    inner: Mutex<Inner>,
    // TODO: add Condvars as needed:
    customer_cv: Condvar,   // barber waits on this
    barber_cv: Condvar,     // customer waits for barber to be ready
    done_cust_cv: Condvar,  // barber waits for customer-is-done signal
    done_barb_cv: Condvar,  // customer waits for barber-is-done signal
    stop: AtomicBool,
}

struct Inner {
    customers: usize,
    // TODO: add flags/counters to coordinate the 4 rendezvous events:
    customer_ready: bool,
    barber_ready: bool,
    customer_done: bool,
    barber_done: bool,
}

impl Barbershop {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                customers: 0,
                customer_ready: false,
                barber_ready: false,
                customer_done: false,
                barber_done: false,
            }),
            customer_cv: Condvar::new(),
            barber_cv: Condvar::new(),
            done_cust_cv: Condvar::new(),
            done_barb_cv: Condvar::new(),
            stop: AtomicBool::new(false),
        }
    }

    /// Returns true if served, false if balked.
    pub fn customer(&self, _id: i32) -> bool {
        // ================================================================
        // TODO:
        //   - lock inner
        //   - if customers == CHAIRS: unlock, return false
        //   - customers += 1
        //   - signal barber ("customer ready"): set flag, notify customer_cv
        //   - wait for barber ready: wait on barber_cv until barber_ready
        //   - (haircut — sleep while holding nothing)
        //   - signal done: set customer_done, notify done_cust_cv
        //   - wait for barber_done
        //   - customers -= 1
        //   - return true
        // ================================================================
        let _ = &self.customer_cv;
        let _ = &self.barber_cv;
        let _ = &self.done_cust_cv;
        let _ = &self.done_barb_cv;
        let _ = &self.inner;
        false
    }

    pub fn barber(&self) {
        while !self.stop.load(Ordering::SeqCst) {
            // ================================================================
            // TODO:
            //   - lock inner
            //   - wait on customer_cv until customer_ready OR stop
            //   - clear customer_ready; set barber_ready; notify barber_cv
            //   - unlock, cut hair (sleep)
            //   - lock, wait on done_cust_cv until customer_done
            //   - clear customer_done, set barber_done, notify done_barb_cv
            // ================================================================
            break;
        }
    }

    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.customer_cv.notify_all();
    }
}

fn main() {
    let shop = Arc::new(Barbershop::new());
    let shop_for_barber = Arc::clone(&shop);
    let barber_thread = thread::spawn(move || shop_for_barber.barber());

    let served = Arc::new(AtomicI32::new(0));
    let balked = Arc::new(AtomicI32::new(0));

    let mut customer_handles = Vec::new();
    let mut rng = rand::thread_rng();
    for i in 0..TOTAL_CUSTOMERS {
        thread::sleep(Duration::from_millis(rng.gen_range(0..30)));
        let s = Arc::clone(&shop);
        let served = Arc::clone(&served);
        let balked = Arc::clone(&balked);
        customer_handles.push(thread::spawn(move || {
            if s.customer(i) {
                served.fetch_add(1, Ordering::SeqCst);
            } else {
                balked.fetch_add(1, Ordering::SeqCst);
            }
        }));
    }
    for h in customer_handles {
        h.join().unwrap();
    }

    shop.shutdown();
    barber_thread.join().unwrap();

    let s = served.load(Ordering::SeqCst);
    let b = balked.load(Ordering::SeqCst);
    println!(
        "Served={} Balked={} Total={} (expected {})",
        s,
        b,
        s + b,
        TOTAL_CUSTOMERS
    );
    assert_eq!(s + b, TOTAL_CUSTOMERS, "lost customers — bug!");
}
