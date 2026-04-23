// Readers-Writers — starter template (Rust)
//
// Scenario: in-memory KV cache.
//   - get(k) is a READ — many concurrent readers OK.
//   - set(k, v) is a WRITE — exclusive.
//
// Three implementations to compare (at minimum):
//   1. std::sync::RwLock (below — kept COMPLETE as the reference baseline).
//   2. Plain std::sync::Mutex. Sometimes wins when critical sections are tiny
//      — RwLock has more atomics per call, overhead is real.
//   3. Hand-rolled lightswitch (lecture slide 10-11) with Mutex+Condvar.
//      Reproduce writer starvation; then fix with the turnstile pattern
//      from slide 12.
//
// Invariants (enforced by Counters): exactly one of
//   - writers == 0 && readers >= 0   (readers can coexist)
//   - writers == 1 && readers == 0   (writer exclusive)
//
// Rust note: the borrow checker + Send/Sync catch a lot here — if it
// compiles, you've avoided data races on primitives. Logic errors
// (deadlock, starvation, broken invariant) are still all yours.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::thread;
use std::time::Instant;

const NUM_READERS: usize = 8;
const NUM_WRITERS: usize = 2;
const OPS_PER_THREAD: usize = 1000;

// --------------------------------------------------------------------------
// Runtime invariant tracker. Wrap every critical section.
// --------------------------------------------------------------------------
struct Counters {
    readers: AtomicI32,
    writers: AtomicI32,
}

impl Counters {
    fn new() -> Self {
        Self {
            readers: AtomicI32::new(0),
            writers: AtomicI32::new(0),
        }
    }
    fn enter_read(&self) {
        self.readers.fetch_add(1, Ordering::SeqCst);
        assert_eq!(
            self.writers.load(Ordering::SeqCst),
            0,
            "reader entered while writer active"
        );
    }
    fn exit_read(&self) {
        self.readers.fetch_sub(1, Ordering::SeqCst);
    }
    fn enter_write(&self) {
        self.writers.fetch_add(1, Ordering::SeqCst);
        let w = self.writers.load(Ordering::SeqCst);
        let r = self.readers.load(Ordering::SeqCst);
        assert!(
            w == 1 && r == 0,
            "writer entered with writers={w} readers={r}"
        );
    }
    fn exit_write(&self) {
        self.writers.fetch_sub(1, Ordering::SeqCst);
    }
}

// Trait so the benchmark harness is generic over every impl.
pub trait Cache: Send + Sync {
    fn get(&self, k: &str) -> String;
    fn set(&self, k: String, v: String);
}

// ==========================================================================
// Impl 1: std::sync::RwLock — COMPLETE. Reference baseline.
// ==========================================================================
pub struct KVCacheRw {
    map: RwLock<HashMap<String, String>>,
    counters: Counters,
}

impl KVCacheRw {
    pub fn new() -> Self {
        Self {
            map: RwLock::new(HashMap::new()),
            counters: Counters::new(),
        }
    }
}

impl Cache for KVCacheRw {
    fn get(&self, k: &str) -> String {
        let guard = self.map.read().unwrap();
        self.counters.enter_read();
        let out = guard.get(k).cloned().unwrap_or_default();
        self.counters.exit_read();
        out
    }
    fn set(&self, k: String, v: String) {
        let mut guard = self.map.write().unwrap();
        self.counters.enter_write();
        guard.insert(k, v);
        self.counters.exit_write();
    }
}

// ==========================================================================
// Impl 2: hand-rolled lightswitch. TODO.
//
// Suggested fields (add what your design actually needs):
//   state: Mutex<RWState>   where RWState = { readers_in: usize, writer_in: bool }
//   room_empty: Condvar     // writers wait while a reader or writer occupies
//   (optional) turnstile: Mutex<()>  // for the no-starve variant (slide 12)
// ==========================================================================
pub struct KVCacheHandRolled {
    map: Mutex<HashMap<String, String>>,
    counters: Counters,
    // TODO: readers_in counter behind a Mutex, Condvar roomEmpty, etc.
    // Left unused so the file compiles; remove the `#[allow]` when you
    // start using them.
    #[allow(dead_code)]
    room_empty: Condvar,
}

impl KVCacheHandRolled {
    pub fn new() -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
            counters: Counters::new(),
            room_empty: Condvar::new(),
        }
    }
}

impl Cache for KVCacheHandRolled {
    fn get(&self, _k: &str) -> String {
        // TODO: implement the "read lock" side of the lightswitch.
        //   - first reader entering takes roomEmpty (blocks writers)
        //   - subsequent readers just bump the counter
        //   - last reader out releases roomEmpty
        //
        // The Counters wrap below MUST be inside the critical section so
        // the invariant assertions see a coherent state.
        self.counters.enter_read();
        let guard = self.map.lock().unwrap();
        let out = guard.get(_k).cloned().unwrap_or_default();
        drop(guard);
        self.counters.exit_read();
        // TODO: release read lock
        out
    }

    fn set(&self, k: String, v: String) {
        // TODO: acquire the "write lock" — exclusive. For the no-starve
        // variant (slide 12), acquire a turnstile first so readers queue
        // up behind a waiting writer, then take roomEmpty.
        self.counters.enter_write();
        let mut guard = self.map.lock().unwrap();
        guard.insert(k, v);
        drop(guard);
        self.counters.exit_write();
        // TODO: release write lock
    }
}

// --------------------------------------------------------------------------
// Generic benchmark harness — times reads + writes across threads.
// --------------------------------------------------------------------------
fn run_benchmark<C: Cache + 'static>(name: &str, cache: Arc<C>) {
    let start = Instant::now();
    let mut handles = Vec::new();

    for r in 0..NUM_READERS {
        let c = Arc::clone(&cache);
        handles.push(thread::spawn(move || {
            for i in 0..OPS_PER_THREAD {
                let _ = c.get(&format!("key{}", (r + i) % 10));
            }
        }));
    }
    for w in 0..NUM_WRITERS {
        let c = Arc::clone(&cache);
        handles.push(thread::spawn(move || {
            for i in 0..OPS_PER_THREAD {
                c.set(format!("key{}", i % 10), format!("v{}", i));
            }
            let _ = w;
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    println!("{:<14} {} ms", name, start.elapsed().as_millis());
}

fn main() {
    run_benchmark("RwLock", Arc::new(KVCacheRw::new()));

    // TODO: once KVCacheHandRolled actually locks, uncomment:
    // run_benchmark("HandRolled", Arc::new(KVCacheHandRolled::new()));
}
