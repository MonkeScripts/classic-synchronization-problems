// Barbershop — tokio async-semaphore template (Rust).
//
// Companion to the Condvar-based `main.rs`. Mirrors the C++ slide-44
// 4-semaphore solution, but using `tokio::sync::Semaphore` from async tasks
// instead of OS threads + Condvars.
//
// Run:
//   cargo run -p barbershop --bin tokio --release
//
// Tokio Semaphore note: permits are RAII — dropping a `SemaphorePermit`
// returns it. To use it as a classical Dijkstra P/V semaphore:
//   - P (wait):   let p = sem.acquire().await.unwrap(); p.forget();
//   - V (signal): sem.add_permits(1);
// Forgetting the permit is what makes signals "stick" so the next P sees them.
//
// `customers` is guarded by `std::sync::Mutex` (not `tokio::sync::Mutex`)
// because the critical section never spans an `.await` — the tokio docs
// explicitly recommend std for this case.
//
// Invariants:
//   - served + balked == TOTAL_CUSTOMERS
//   - customers never exceeds CHAIRS

use rand::Rng;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Semaphore;

const CHAIRS: usize = 3;
const TOTAL_CUSTOMERS: i32 = 30;

pub struct Barbershop {
    customers: Mutex<usize>,
    customer_sem: Semaphore,   // barber P's; customer V's
    barber_sem: Semaphore,     // customer P's; barber V's
    customer_done: Semaphore,  // barber P's; customer V's
    barber_done: Semaphore,    // customer P's; barber V's
    stop: AtomicBool,
}

impl Barbershop {
    pub fn new() -> Self {
        Self {
            customers: Mutex::new(0),
            customer_sem: Semaphore::new(0),
            barber_sem: Semaphore::new(0),
            customer_done: Semaphore::new(0),
            barber_done: Semaphore::new(0),
            stop: AtomicBool::new(false),
        }
    }

    /// Returns true if served, false if balked.
    pub async fn customer(&self, _id: i32) -> bool {
        {
            let mut num_customers = self.customers.lock().unwrap();
            if *num_customers == CHAIRS {
                return false;
            }
            *num_customers += 1;
        }
        self.customer_sem.add_permits(1);
        self.barber_sem.acquire().await.unwrap().forget();
        tokio::time::sleep(Duration::from_millis(2)).await; //cut hair
        self.barber_done.acquire().await.unwrap().forget();
        self.customer_done.add_permits(1);
        {
            let mut num_customers = self.customers.lock().unwrap();
            *num_customers -= 1;
        }
        true
    }

    pub async fn barber(&self) {
        while !self.stop.load(Ordering::SeqCst) {
            self.customer_sem.acquire().await.unwrap().forget();
            if (self.stop.load(Ordering::SeqCst)){
                break;
            }
            self.barber_sem.add_permits(1);
            tokio::time::sleep(Duration::from_millis(2)).await; //cut hair
            self.barber_done.add_permits(1);
            self.customer_done.acquire().await.unwrap().forget();
        }
    }

    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.customer_sem.add_permits(1);
    }
}

#[tokio::main]
async fn main() {
    let shop = Arc::new(Barbershop::new());
    let shop_for_barber = Arc::clone(&shop);
    let barber_task = tokio::spawn(async move { shop_for_barber.barber().await });

    let served = Arc::new(AtomicI32::new(0));
    let balked = Arc::new(AtomicI32::new(0));

    let mut handles = Vec::new();
    let mut rng = rand::thread_rng();
    for i in 0..TOTAL_CUSTOMERS {
        let delay = rng.gen_range(0..30);
        tokio::time::sleep(Duration::from_millis(delay)).await;
        let s = Arc::clone(&shop);
        let served = Arc::clone(&served);
        let balked = Arc::clone(&balked);
        handles.push(tokio::spawn(async move {
            if s.customer(i).await {
                served.fetch_add(1, Ordering::SeqCst);
            } else {
                balked.fetch_add(1, Ordering::SeqCst);
            }
        }));
    }
    for h in handles {
        h.await.unwrap();
    }

    shop.shutdown();
    barber_task.await.unwrap();

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
