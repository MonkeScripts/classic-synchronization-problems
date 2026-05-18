// Multi-doctor clinic — Tier 2 Q19 — TEMPLATE
//
// M = 3 doctors share a waiting room of K = 5 chairs. Patients arrive at
// random intervals (mean ~5ms). If a chair is free, the patient sits and
// waits for ANY doctor; if all chairs are taken, the patient BALKS (leaves).
// Doctors pick the next waiting patient (FIFO). Each consultation takes
// 5–20ms.
//
// Run:
//   cargo run -p multi_doctor_clinic --release
//
// =============================================================================
// SUGGESTED DESIGN (R16 ⊕ R13 — barbershop with multi-consumer mpsc):
//
//   - mpsc::channel::<oneshot::Sender<()>>(K_CHAIRS)
//     ↑ the waiting room. Capacity = K. Each in-flight message is one
//       seated patient's reply line.
//
//   - Patient: create (done_tx, done_rx) oneshot pair; chair_tx.try_send(done_tx).
//       Ok(())               → seated; await done_rx → served.
//       Err(Full)            → no chair → BALK.
//       Err(Closed)          → shop already shut → shouldn't happen here.
//
//   - Doctor: lock the shared receiver, recv() one reply line, RELEASE LOCK,
//     consult, then send () back through the reply line to wake the patient.
//
//   - Receiver-side fan-out: M doctors share ONE receiver via
//     Arc<tokio::sync::Mutex<Receiver<...>>>. Each doctor task holds an
//     Arc::clone — they all access the same Mutex<Receiver>.
//
// LOCK SCOPE — THE LOAD-BEARING DETAIL:
//   The mutex is released BEFORE consult(). Block-scope the lock acquisition:
//
//       let next = {
//           let mut rx = recv_lock.lock().await;
//           rx.recv().await
//       };  // *** lock dropped HERE — before consult ***
//
//   If you hold the lock across consult(), only ONE doctor can be inside the
//   function at a time → throughput collapses from M to 1, regardless of how
//   many doctors you spawn. The harness's MAX_ACTIVE_DOCTORS instrumentation
//   catches this — it should reach >1 under any reasonable load.
//
// MUTEX FLAVOR:
//   `tokio::sync::Mutex`, NOT `std::sync::Mutex`. We hold the lock across
//   `recv().await`. `std::sync::MutexGuard` is `!Send`, so a future holding
//   one across `.await` won't be `tokio::spawn`-able.
//
// SHUTDOWN:
//   After all patient tasks have run to completion (each either served or
//   balked), drop the chair_tx held by main. That's the LAST sender → channel
//   closes → each doctor's `recv().await` returns None → doctors exit.
// =============================================================================
//
// Invariants enforced by the harness:
//   - ACTIVE_DOCTORS <= M_DOCTORS (hard fail if violated)
//   - served + balked == TOTAL_PATIENTS
//   - MAX_ACTIVE_DOCTORS > 1 (warning if not — strong signal of lock-scope bug)

use rand::Rng;
use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, Mutex};

const M_DOCTORS: usize = 3;
const K_CHAIRS: usize = 5;
const TOTAL_PATIENTS: i32 = 50;

// ----- Invariant tracking (do not touch) -----
static ACTIVE_DOCTORS: AtomicUsize = AtomicUsize::new(0);
static MAX_ACTIVE_DOCTORS: AtomicUsize = AtomicUsize::new(0);

fn update_max(now: usize) {
    let mut cur = MAX_ACTIVE_DOCTORS.load(Ordering::SeqCst);
    while now > cur {
        match MAX_ACTIVE_DOCTORS.compare_exchange(cur, now, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => break,
            Err(actual) => cur = actual,
        }
    }
}

/// The "actual work" — call from the doctor task INSIDE the consultation.
/// DO NOT modify.
async fn consult(_doc_id: usize) {
    let now = ACTIVE_DOCTORS.fetch_add(1, Ordering::SeqCst) + 1;
    assert!(
        now <= M_DOCTORS,
        "INVARIANT: active_doctors={} > M_DOCTORS={}",
        now,
        M_DOCTORS
    );
    update_max(now);

    let ms = {
        let mut rng = rand::thread_rng();
        rng.gen_range(5..21)
    };
    tokio::time::sleep(Duration::from_millis(ms)).await;

    ACTIVE_DOCTORS.fetch_sub(1, Ordering::SeqCst);
}

// =============================================================================
// TODO: implement doctor.
//
// Signature hint:
//   async fn doctor(
//       id: usize,
//       recv_lock: Arc<Mutex<mpsc::Receiver<oneshot::Sender<()>>>>,
//   )
//
// Body (use this exact lock-scope shape):
//
//   loop {
//       let next = {
//           let mut rx = recv_lock.lock().await;
//           rx.recv().await                                   // INSIDE the lock
//       };  // *** lock released HERE — BEFORE consult ***
//       match next {
//           Some(done_tx) => {
//               consult(id).await;                            // OUTSIDE the lock
//               let _ = done_tx.send(());                     // wake patient (Err if cancelled)
//           }
//           None => return,                                   // channel closed → shutdown
//       }
//   }
// =============================================================================
async fn doctor(
    id: usize,
    recv_lock: Arc<Mutex<mpsc::Receiver<oneshot::Sender<()>>>>,
) {
    loop {
        let item = {
            let mut rx = recv_lock.lock().await;
            rx.recv().await
        };
        match item {
            Some(done_tx) => {
                consult(id).await;
                let _ = done_tx.send(());
            }
            None => break,
        }
    }
}

// =============================================================================
// TODO: implement patient_action.
//
// Returns: true if served, false if balked.
//
// Signature hint:
//   async fn patient_action(
//       id: i32,
//       chair_tx: mpsc::Sender<oneshot::Sender<()>>,
//   ) -> bool
//
// Body:
//   - let (done_tx, done_rx) = oneshot::channel();
//   - match chair_tx.try_send(done_tx) {
//         Ok(()) => { done_rx.await.unwrap(); return true; }      // served
//         Err(mpsc::error::TrySendError::Full(_)) => { return false; }  // balked
//         Err(mpsc::error::TrySendError::Closed(_)) => panic!("shop closed prematurely"),
//     }
// =============================================================================
async fn patient_action(_id: i32, chair_tx: mpsc::Sender<oneshot::Sender<()>>) -> bool {
    let (done_tx, done_rx) = oneshot::channel();
    match chair_tx.try_send(done_tx) {
        Ok(()) => {
            done_rx.await.unwrap();
            true
        }
        Err(mpsc::error::TrySendError::Full(_)) => false,
        Err(mpsc::error::TrySendError::Closed(_)) => panic!("shop closed prematurely"),
    }
}

#[tokio::main]
async fn main() {

    let start = std::time::Instant::now();
    let served = Arc::new(AtomicI32::new(0));
    let balked = Arc::new(AtomicI32::new(0));

    let (chair_tx, chair_rx) = mpsc::channel::<oneshot::Sender<()>>(K_CHAIRS);
    let recv_lock = Arc::new(Mutex::new(chair_rx));

    let mut doctor_handles = Vec::new();
    for id in 0..M_DOCTORS {
        let lock = Arc::clone(&recv_lock);
        doctor_handles.push(tokio::spawn(async move {
            doctor(id, lock).await;
        }));
    }

    let mut patient_handles = Vec::new();
    for id in 0..TOTAL_PATIENTS {
        let ct = chair_tx.clone();
        let sv = Arc::clone(&served);
        let bk = Arc::clone(&balked);
        patient_handles.push(tokio::spawn(async move {
            // stagger arrival so the waiting room actually fills
            let delay = {
                let mut rng = rand::thread_rng();
                rng.gen_range(0..10)
            };
            tokio::time::sleep(Duration::from_millis(delay)).await;

            if patient_action(id, ct).await {
                sv.fetch_add(1, Ordering::SeqCst);
            } else {
                bk.fetch_add(1, Ordering::SeqCst);
            }
        }));
    }

    // Patients first — they each finish (served or balked) and drop their tx clones.
    for h in patient_handles {
        h.await.unwrap();
    }
    // Now drop main's chair_tx — last sender → channel closes → doctors exit on None.
    drop(chair_tx);
    for h in doctor_handles {
        h.await.unwrap();
    }

    let elapsed = start.elapsed();

    // ----- Summary (do not modify) -----
    let s = served.load(Ordering::SeqCst);
    let b = balked.load(Ordering::SeqCst);
    let max_active = MAX_ACTIVE_DOCTORS.load(Ordering::SeqCst);

    println!(
        "doctors: {}  chairs: {}  total_patients: {}",
        M_DOCTORS, K_CHAIRS, TOTAL_PATIENTS
    );
    println!("served: {}  balked: {}  total: {}", s, b, s + b);
    println!(
        "max active doctors: {} (target: > 1; if 1 → tight-lock-scope bug)",
        max_active
    );
    println!("time: {:?}", elapsed);

    let mut ok = true;
    if s + b != TOTAL_PATIENTS {
        eprintln!(
            "INVARIANT: served+balked {} != TOTAL_PATIENTS {}",
            s + b,
            TOTAL_PATIENTS
        );
        ok = false;
    }
    if max_active > M_DOCTORS {
        eprintln!(
            "INVARIANT: max_active_doctors {} > M_DOCTORS {}",
            max_active, M_DOCTORS
        );
        ok = false;
    }
    if max_active < 2 {
        eprintln!(
            "WARNING: max_active_doctors only {} — likely lock-scope bug \
             (should be > 1 under any reasonable load)",
            max_active
        );
        // not a hard fail; could legitimately be 1 if load is extremely light
    }
    if !ok {
        std::process::exit(1);
    }
}
