// H2O Water Factory — tokio template (Rust)
//
// Scenario (lecture T7.1): hydrogen and oxygen atoms arrive at a factory.
// When 2 H + 1 O are present, they bond into a water molecule.
//
// Run:
//   cargo run -p h2o --release
//
// Strategies (pick one, then try a second):
//   1. tokio::sync::Semaphore::new(2) (H) + Semaphore::new(1) (O)
//      + tokio::sync::Barrier::new(3). Mirrors C++ WaterFactory3.
//      Each acquirer holds its semaphore permit ACROSS the barrier+bond,
//      then releases on exit. Use `permit.forget()` + `add_permits(1)` if
//      you want classical-Dijkstra semantics (see barbershop notes).
//   2. Daemon task with mpsc channels (Go-style "two-phase commit"):
//        precommit: 2 H + 1 O each send their oneshot::Sender to the daemon
//        commit:    daemon sends () to all 3
//        postcommit: each atom sends () back; daemon waits for all 3
//      The daemon owns the protocol; atoms just submit themselves and wait.
//   3. Leader election with oxygen as leader (Go-style with a Mutex/permit).
//      Oxygen takes a 1-permit "leader" semaphore; collects 2 H precommits
//      via a shared mpsc; sends them go; bonds; waits for both H to finish;
//      releases the leader permit.
//
// Invariants enforced by the harness:
//   - H_IN_BOND <= 2  (assertion fires on increment past 2 — catches ozone)
//   - O_IN_BOND <= 1  (catches double-oxygen)
//   - MAX_TOTAL_IN_BOND should reach 3 (catches "no barrier" — atoms bonding solo)
//   - H_TOTAL_BONDS == 2 * N_MOLECULES, O_TOTAL_BONDS == N_MOLECULES

use rand::Rng;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Semaphore, TryAcquireError, Barrier};


const N_MOLECULES: usize = 50;
const N_HYDROGEN: usize = 2 * N_MOLECULES;
const N_OXYGEN: usize = N_MOLECULES;

// ----- Invariant tracking (do not touch) -----
static H_IN_BOND: AtomicUsize = AtomicUsize::new(0);
static O_IN_BOND: AtomicUsize = AtomicUsize::new(0);
static MAX_TOTAL_IN_BOND: AtomicUsize = AtomicUsize::new(0);
static H_TOTAL_BONDS: AtomicUsize = AtomicUsize::new(0);
static O_TOTAL_BONDS: AtomicUsize = AtomicUsize::new(0);

fn update_max_total() {
    let total = H_IN_BOND.load(Ordering::SeqCst) + O_IN_BOND.load(Ordering::SeqCst);
    let mut cur = MAX_TOTAL_IN_BOND.load(Ordering::SeqCst);
    while total > cur {
        match MAX_TOTAL_IN_BOND.compare_exchange(cur, total, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => break,
            Err(actual) => cur = actual,
        }
    }
}

async fn bond_h(_id: usize) {
    let h = H_IN_BOND.fetch_add(1, Ordering::SeqCst) + 1;
    assert!(h <= 2, "ozone! {} H atoms in bond simultaneously", h);
    update_max_total();
    tokio::time::sleep(Duration::from_millis(2)).await;
    update_max_total();
    tokio::time::sleep(Duration::from_millis(1)).await;
    H_IN_BOND.fetch_sub(1, Ordering::SeqCst);
    H_TOTAL_BONDS.fetch_add(1, Ordering::SeqCst);
}

async fn bond_o(_id: usize) {
    let o = O_IN_BOND.fetch_add(1, Ordering::SeqCst) + 1;
    assert!(o <= 1, "more than 1 O in bond ({} O atoms)", o);
    update_max_total();
    tokio::time::sleep(Duration::from_millis(2)).await;
    update_max_total();
    tokio::time::sleep(Duration::from_millis(1)).await;
    O_IN_BOND.fetch_sub(1, Ordering::SeqCst);
    O_TOTAL_BONDS.fetch_add(1, Ordering::SeqCst);
}

// ==========================================================================
// TODO: Implement WaterFactory.
//
// API contract:
//   hydrogen(id).await: block until grouped with 1 H + 1 O, then call bond_h(id).await
//   oxygen(id).await:   block until grouped with 2 H,         then call bond_o(id).await
// ==========================================================================
pub struct WaterFactory {
    // TODO: fields
    oxygen_sem: Semaphore,
    hydrogen_sem: Semaphore,
    barrier: Barrier,
    
}

impl WaterFactory {
    pub fn new() -> Self {
        Self {
            oxygen_sem: Semaphore::new(1),
            hydrogen_sem: Semaphore::new(2),
            barrier: Barrier::new(3),
        }
    }

    pub async fn hydrogen(&self, id: usize) {
        // TODO: coordinate, then bond_h(id).await
        self.hydrogen_sem.acquire().await.unwrap().forget();
        self.barrier.wait().await;
        bond_h(id).await;
        self.hydrogen_sem.add_permits(1);
    }

    pub async fn oxygen(&self, id: usize) {
        // TODO: coordinate, then bond_o(id).await\
        self.oxygen_sem.acquire().await.unwrap().forget();
        self.barrier.wait().await;
        bond_o(id).await;
        self.oxygen_sem.add_permits(1);
    }
}

#[tokio::main]
async fn main() {
    let factory = Arc::new(WaterFactory::new());
    let mut handles = Vec::new();
    let mut rng = rand::thread_rng();

    let start = std::time::Instant::now();
    for i in 0..N_HYDROGEN {
        tokio::time::sleep(Duration::from_millis(rng.gen_range(0..5))).await;
        let f = Arc::clone(&factory);
        handles.push(tokio::spawn(async move { f.hydrogen(i).await }));
    }
    for i in 0..N_OXYGEN {
        tokio::time::sleep(Duration::from_millis(rng.gen_range(0..5))).await;
        let f = Arc::clone(&factory);
        handles.push(tokio::spawn(async move { f.oxygen(i).await }));
    }
    for h in handles {
        h.await.unwrap();
    }
    let elapsed = start.elapsed();

    let h_bonds = H_TOTAL_BONDS.load(Ordering::SeqCst);
    let o_bonds = O_TOTAL_BONDS.load(Ordering::SeqCst);
    let max_total = MAX_TOTAL_IN_BOND.load(Ordering::SeqCst);

    println!(
        "H bonds: {}/{}  O bonds: {}/{}  max simultaneous in bond: {}  time: {:?}",
        h_bonds, N_HYDROGEN, o_bonds, N_OXYGEN, max_total, elapsed
    );

    let mut ok = true;
    if h_bonds != N_HYDROGEN {
        eprintln!("MISSING H BONDS");
        ok = false;
    }
    if o_bonds != N_OXYGEN {
        eprintln!("MISSING O BONDS");
        ok = false;
    }
    if max_total != 3 {
        eprintln!("max_total_in_bond should be 3 — atoms are bonding without all 3 being present");
        ok = false;
    }
    if !ok {
        std::process::exit(1);
    }
}
