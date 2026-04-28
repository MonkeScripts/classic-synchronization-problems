// H2O Water Factory — daemon-task tokio template (Rust)
//
// Companion to src/main.rs (semaphore + barrier strategy). Same problem,
// different shape: a daemon TASK orchestrates each molecule, and atoms
// just submit themselves and wait. Mirrors the Go daemon strategy.
//
// Run:
//   cargo run -p h2o --bin daemon --release
//
// Strategy:
//   - WaterFactory holds two mpsc senders (one for H, one for O).
//   - new() spawns a tokio task running the daemon loop; ownership of the
//     corresponding mpsc receivers moves INTO that task.
//   - Each atom creates a pair of oneshot channels (go + done), wraps them
//     in a Request, sends to the appropriate mpsc, and waits for go.
//   - The daemon collects 2 H + 1 O Requests, sends () on each go channel,
//     waits on each done channel, and loops.
//
// Why TWO oneshots, not one (vs Go's single channel):
//   Go reuses one bidirectional channel for both "go" and "done" because
//   chan struct{} is reusable. Rust's tokio::sync::oneshot is single-shot —
//   you can't fire it twice. So we need two: go_tx/go_rx for daemon→atom,
//   done_tx/done_rx for atom→daemon. Same protocol, more types.
//
// Why oneshot and not a second mpsc:
//   The done signal is addressed to one specific atom (the daemon's "next
//   loop iteration must wait for THESE three atoms"), not "anyone listening".
//   oneshot is the right primitive for "exactly one message to one receiver".
//
// Trade-offs vs WaterFactory3 (semaphore + barrier in src/main.rs):
//   - More allocation per molecule (oneshots) but the protocol lives in one
//     place (the daemon function).
//   - Daemon task leaks unless you wire up shutdown (same caveat as Go).
//
// Invariants enforced by the harness: same as src/main.rs.

use rand::Rng;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

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
// TODO: Implement WaterFactory using the DAEMON-TASK strategy.
//
// Suggested shape:
//
//   pub struct WaterFactory {
//       h_tx: mpsc::Sender<Request>,
//       o_tx: mpsc::Sender<Request>,
//   }
//
//   impl WaterFactory {
//       pub fn new() -> Self {
//           let (h_tx, h_rx) = mpsc::channel(N_HYDROGEN);
//           let (o_tx, o_rx) = mpsc::channel(N_OXYGEN);
//           tokio::spawn(daemon(h_rx, o_rx));
//           Self { h_tx, o_tx }
//       }
//   }
//
// Daemon loop:
//   async fn daemon(mut h_rx: mpsc::Receiver<Request>,
//                   mut o_rx: mpsc::Receiver<Request>) {
//       loop {
//           // precommit: collect 2 H + 1 O
//           let h1 = h_rx.recv().await.unwrap();
//           let h2 = h_rx.recv().await.unwrap();
//           let o  = o_rx.recv().await.unwrap();
//           // commit: tell each one to go
//           h1.go.send(()).unwrap();
//           h2.go.send(()).unwrap();
//           o.go.send(()).unwrap();
//           // postcommit: wait for each one to be done
//           h1.done.await.unwrap();
//           h2.done.await.unwrap();
//           o.done.await.unwrap();
//       }
//   }
//
// hydrogen() / oxygen():
//   let (go_tx, go_rx) = oneshot::channel();
//   let (done_tx, done_rx) = oneshot::channel();
//   self.h_tx.send(Request { go: go_tx, done: done_rx }).await.unwrap();
//   go_rx.await.unwrap();        // wait for "go"
//   bond_h(id).await;
//   done_tx.send(()).unwrap();   // signal "done"
//
// Note: tokio::spawn inside new() requires being called from within a tokio
// runtime — fine here because main() is #[tokio::main], but you couldn't
// build a WaterFactory from a sync context.
// ==========================================================================
struct Request {
    go: oneshot::Sender<()>,
    done: oneshot::Receiver<()>,
}

pub struct WaterFactory {
    h_tx: mpsc::Sender<Request>,
    o_tx: mpsc::Sender<Request>,
}

async fn manager_loop(
    mut h_rx: mpsc::Receiver<Request>,
    mut o_rx: mpsc::Receiver<Request>,
) {
    loop {
        // precommit: collect 2 H + 1 O
        let h1 = h_rx.recv().await.unwrap();
        let h2 = h_rx.recv().await.unwrap();
        let o = o_rx.recv().await.unwrap();
        // commit: tell each one to go
        h1.go.send(()).unwrap();
        h2.go.send(()).unwrap();
        o.go.send(()).unwrap();
        // postcommit: wait for each one to be done
        h1.done.await.unwrap();
        h2.done.await.unwrap();
        o.done.await.unwrap();
    }
}

impl WaterFactory {
    pub fn new() -> Self {
        let (h_tx, h_rx) = mpsc::channel(N_HYDROGEN);
        let (o_tx, o_rx) = mpsc::channel(N_OXYGEN);
        tokio::spawn(manager_loop(h_rx, o_rx));
        Self { h_tx, o_tx }
    }

    pub async fn hydrogen(&self, id: usize) {
        let (go_tx, go_rx) = oneshot::channel();
        let (done_tx, done_rx) = oneshot::channel();
        self.h_tx.send(Request { go: go_tx, done: done_rx }).await.unwrap();
        go_rx.await.unwrap();
        bond_h(id).await;
        done_tx.send(()).unwrap();
    }

    pub async fn oxygen(&self, id: usize) {
        let (go_tx, go_rx) = oneshot::channel();
        let (done_tx, done_rx) = oneshot::channel();
        self.o_tx.send(Request { go: go_tx, done: done_rx }).await.unwrap();
        go_rx.await.unwrap();
        bond_o(id).await;
        done_tx.send(()).unwrap();
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
