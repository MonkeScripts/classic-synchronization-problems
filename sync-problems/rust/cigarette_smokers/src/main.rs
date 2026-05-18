// Cigarette Smokers — Tier 2 Q10 (Patil, 1971) — TEMPLATE
//
// Three smokers + one agent. Each smoker has unlimited supply of ONE
// ingredient (tobacco / paper / matches). To smoke, you need all three.
// The agent has all three; each round picks 2 ingredients to "place on the
// table" and signals the smoker holding the THIRD (NOT-placed) ingredient
// to pick them up and smoke. Run for N_ROUNDS; verify each smoker smoked
// roughly equally.
//
// Run:
//   cargo run -p cigarette_smokers --release
//
// =============================================================================
// SUGGESTED DESIGN (daemon shape — same as h2o daemon, EXAM_GUIDE_PER_PROBLEM.md §6.6):
//
// Topology:
//   - 3 dedicated `tokio::sync::mpsc::channel::<()>(1)`, one per smoker.
//     Agent owns all 3 tx; each smoker owns one rx.
//     Sending on smoker_tx[i] is the *addressed* wake-up for smoker i.
//     Receiving a message on your own channel IS the proof "your ingredient
//     is the third" — no payload needed.
//   - 1 shared `tokio::sync::mpsc::channel::<()>(1)` for done. Smokers each
//     clone done_tx; agent holds done_rx. Awaiting one done message between
//     rounds is the round barrier — without it, agent races into round N+1
//     before smoker N has finished smoking → invariant fires.
//
// Why NOT broadcast: would wake all 3 smokers and force them to self-filter
// on a "missing ingredient" payload. 2/3 smokers wake every round to do
// nothing. Addressed mpsc keeps smokers stateless.
//
// Why NOT oneshot per round (instead of persistent mpsc): oneshot::Sender is
// not Clone — fanning it to 3 smokers requires Arc<Mutex<Option<>>>. Persistent
// mpsc<()> is the same shape h2o daemon uses.
//
// Shutdown: when agent_loop returns, the smoker_tx array is dropped. Each
// smoker's rx then returns None on the next recv(); the while-let exits;
// smokers return. main() awaits all smoker JoinHandles after the agent.
// =============================================================================
//
// Invariants enforced by the harness:
//   - SIMULTANEOUS_SMOKES <= 1 (asserted on entry to smoke())
//   - max_simultaneous == 1 at the end (catches "agent races ahead of done")
//   - total smokes == N_ROUNDS

use rand::Rng;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;

const N_ROUNDS: usize = 30;
const N_SMOKERS: usize = 3;
const INGREDIENTS: [&str; N_SMOKERS] = ["tobacco", "paper", "matches"];

// ----- Invariant tracking (do not touch) -----
static SIMULTANEOUS_SMOKES: AtomicUsize = AtomicUsize::new(0);
static MAX_SIMULTANEOUS: AtomicUsize = AtomicUsize::new(0);
static SMOKE_COUNTS: [AtomicUsize; N_SMOKERS] = [
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
];

fn update_max(now: usize) {
    let mut cur = MAX_SIMULTANEOUS.load(Ordering::SeqCst);
    while now > cur {
        match MAX_SIMULTANEOUS.compare_exchange(cur, now, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => break,
            Err(actual) => cur = actual,
        }
    }
}

/// The "actual work." Call this from your smoker task. DO NOT modify.
async fn smoke(id: usize) {
    let now = SIMULTANEOUS_SMOKES.fetch_add(1, Ordering::SeqCst) + 1;
    assert!(now <= 1, "INVARIANT: {} smokers smoking simultaneously", now);
    update_max(now);

    let ms = {
        let mut rng = rand::thread_rng();
        rng.gen_range(1..5)
    };
    tokio::time::sleep(Duration::from_millis(ms)).await;

    SIMULTANEOUS_SMOKES.fetch_sub(1, Ordering::SeqCst);
    SMOKE_COUNTS[id].fetch_add(1, Ordering::SeqCst);
}

// =============================================================================
// TODO: implement smoker_loop.
//
// Signature hint:
//   async fn smoker_loop(id: usize, mut rx: mpsc::Receiver<()>, done_tx: mpsc::Sender<()>)
//
// Body:
//   - Loop while rx.recv().await is Some(()):
//       - call smoke(id).await
//       - send () on done_tx (must await — it's an async send)
//   - When rx.recv() returns None, the agent has dropped its tx → exit.
// =============================================================================
async fn smoker_loop(id: usize, mut rx: mpsc::Receiver<()>, done_tx: mpsc::Sender<()>) {
    while let Some(()) = rx.recv().await {
        smoke(id).await;
        done_tx.send(()).await.expect("agent dropped done_rx");
    }
    // rx closed → agent dropped its smoker_tx → shutdown
}

// =============================================================================
// TODO: implement agent_loop.
//
// Signature hint:
//   async fn agent_loop(
//       smoker_tx: [mpsc::Sender<()>; N_SMOKERS],
//       mut done_rx: mpsc::Receiver<()>,
//       rounds: usize,
//   )
//
// Body:
//   - For each of `rounds` rounds:
//       - pick `third` uniformly at random in 0..N_SMOKERS
//         (use `rand::thread_rng().gen_range(0..N_SMOKERS)`; scope the RNG
//         in a `{ ... }` block so it's dropped before the next .await)
//       - smoker_tx[third].send(()).await
//       - done_rx.recv().await  (wait for the smoker to finish — round barrier)
//   - On return, smoker_tx is dropped → smokers exit cleanly.
// =============================================================================
async fn agent_loop(
    smoker_tx: [mpsc::Sender<()>; N_SMOKERS],
    mut done_rx: mpsc::Receiver<()>,
    rounds: usize,
) {
    for _ in 0..rounds {
        let chosen_smoker = rand::thread_rng().gen_range(0..N_SMOKERS);
        smoker_tx[chosen_smoker]
            .send(())
            .await
            .expect("smoker exited unexpectedly");
        done_rx.recv().await.expect("smoker channel closed before done");
    }
    // smoker_tx array dropped on return → smokers' rx returns None → exit
}

#[tokio::main]
async fn main() {
    // =========================================================================
    // TODO: set up the topology and spawn tasks.
    //
    // 1. Create the 3 dedicated wake-up channels:
    //      let (tx_i, rx_i) = mpsc::channel(1);   // for each smoker i
    //    Collect all 3 tx into a Vec, all 3 rx into another Vec.
    //
    // 2. Create the shared done channel:
    //      let (done_tx, done_rx) = mpsc::channel(1);
    //
    // 3. Spawn 3 smoker tasks. Each gets:
    //      - its id (0..N_SMOKERS)
    //      - its own rx (move it into the task)
    //      - a `done_tx.clone()`
    //    Collect the JoinHandles in a Vec.
    //
    // 4. drop(done_tx) — smokers hold the only remaining clones now.
    //
    // 5. Convert the Vec of smoker txs to a fixed array:
    //      let smoker_tx_arr: [mpsc::Sender<()>; N_SMOKERS] =
    //          smoker_txs.try_into().unwrap();
    //
    // 6. Call agent_loop(smoker_tx_arr, done_rx, N_ROUNDS).await directly
    //    (no spawn needed — main is async). When it returns, smokers will
    //    exit because the array is dropped.
    //
    // 7. Await all smoker JoinHandles.
    // =========================================================================
    let start = std::time::Instant::now();
    let (done_tx, done_rx) = mpsc::channel(1);
    let mut smoker_handles = Vec::new();
    let mut smoker_tx: Vec<mpsc::Sender<()>> = Vec::new();

    for i in 0..N_SMOKERS {
        let (tx_i, rx_i) = mpsc::channel(1);
        smoker_tx.push(tx_i);
        let dt = done_tx.clone();
        smoker_handles.push(tokio::spawn(async move {
            smoker_loop(i, rx_i, dt).await
        }));
    }
    drop(done_tx); // smokers hold the only remaining clones

    let smoker_tx_arr: [mpsc::Sender<()>; N_SMOKERS] = smoker_tx
        .try_into()
        .expect("smoker_tx vec must have exactly N_SMOKERS elements");

    agent_loop(smoker_tx_arr, done_rx, N_ROUNDS).await;
    // agent returned → smoker_tx_arr dropped → smokers' rx see channel-closed

    for h in smoker_handles {
        h.await.unwrap();
    }

    let elapsed = start.elapsed();

    // ----- Summary (do not modify) -----
    let total: usize = SMOKE_COUNTS.iter().map(|c| c.load(Ordering::SeqCst)).sum();
    let max_sim = MAX_SIMULTANEOUS.load(Ordering::SeqCst);

    println!("rounds: {}  time: {:?}", N_ROUNDS, elapsed);
    println!("max simultaneous smokers: {}", max_sim);
    println!("smoke counts:");
    for (id, c) in SMOKE_COUNTS.iter().enumerate() {
        println!(
            "  smoker {} ({}): {}",
            id,
            INGREDIENTS[id],
            c.load(Ordering::SeqCst)
        );
    }
    println!("total smokes: {} / {}", total, N_ROUNDS);

    let mut ok = true;
    if total != N_ROUNDS {
        eprintln!("INVARIANT: total smokes {} != N_ROUNDS {}", total, N_ROUNDS);
        ok = false;
    }
    if max_sim != 1 {
        eprintln!("INVARIANT: expected max_simultaneous == 1, got {}", max_sim);
        ok = false;
    }
    if !ok {
        std::process::exit(1);
    }
}
