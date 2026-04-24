// Producer-Consumer (mpsc variant) — starter template (Rust)
//
// Same scenario as src/main.rs (the condvar version). Solve the same
// problem with std::sync::mpsc::sync_channel and observe where the
// complexity moved.
//
// Run:
//     cargo run -p producer_consumer --bin mpsc --release
//
// Things to compare against the condvar version after you finish:
//   - LOC in steady-state vs shutdown.
//   - You don't call `close()` anywhere — what mechanism replaces it?
//   - std mpsc is Multi-Producer SINGLE-Consumer. What do you have to
//     do to share the Receiver across NUM_CONSUMERS threads, and what
//     does that pattern cost you in real consumer concurrency?
//
// Two pitfalls to think about BEFORE you start typing:
//   (A) The channel closes only when ALL senders are dropped. If the
//       original SyncSender lives in main() for the whole run, recv()
//       never errors → consumers hang forever waiting for an item that
//       could still arrive.
//   (B) Receiver is NOT Clone. Sharing it across consumers requires
//       Arc<Mutex<Receiver<T>>>. That works, but only one consumer can
//       hold the mutex at a time, so multi-consumer becomes effectively
//       serial during recv. Worth noting in your writeup.
//
// Stretch / second pass:
//   - Replace std mpsc with `crossbeam-channel` — Receiver IS Clone, so
//     you get true MPMC and can drop the Arc<Mutex<_>> ceremony.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::mpsc::sync_channel;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const BUFFER_SIZE: usize = 8;
const NUM_PRODUCERS: usize = 3;
const NUM_CONSUMERS: usize = 2;
const ITEMS_PER_PRODUCER: i32 = 50;

fn main() {
    let produced = Arc::new(AtomicI32::new(0));
    let consumed = Arc::new(AtomicI32::new(0));

    // ====================================================================
    // TODO:
    //   1. Create the channel:
    //        let (tx, rx) = sync_channel::<String>(BUFFER_SIZE);
    //
    //   2. Wrap rx so NUM_CONSUMERS threads can share it.
    //        (Hint: Receiver is not Clone — see pitfall (B).)
    //
    //   3. Spawn NUM_PRODUCERS producer threads. Each:
    //        - has its own clone of `tx`
    //        - sends format!("P{}#{}", pid, i) for i in 0..ITEMS_PER_PRODUCER
    //        - increments `produced` after each successful send
    //        - thread::sleep(Duration::from_millis((pid as u64) % 3))
    //      Think: what does `tx.send(...)` return, and what should you do
    //      with that Result here?
    //
    //   4. AFTER spawning all producers, drop the original `tx` you held
    //      in main(). (See pitfall (A). Forget this and consumers hang.)
    //
    //   5. Spawn NUM_CONSUMERS consumer threads. Each loops:
    //        - acquire the receiver mutex briefly, recv() one item, release
    //        - on Ok(item):  std::hint::black_box(item); consumed += 1
    //        - on Err(_):    break — the channel is fully closed
    //      Think: what goes wrong if you hold the mutex across the recv()
    //      blocking wait? (Answer: nothing crashes, but think about why
    //      you'd want a small lock scope anyway.)
    //
    //   6. Join everyone, then keep the print + asserts below.
    // ====================================================================

    

    let expected = (NUM_PRODUCERS as i32) * ITEMS_PER_PRODUCER;
    let p = produced.load(Ordering::SeqCst);
    let c = consumed.load(Ordering::SeqCst);
    println!("Produced={} Consumed={} Expected={}", p, c, expected);
    assert_eq!(p, expected, "not all items produced");
    assert_eq!(c, expected, "not all items consumed");
}
