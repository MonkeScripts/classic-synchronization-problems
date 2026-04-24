// Readers-Writers — starter template (Rust, async/tokio variant)
//
// Scenario: in-memory KV cache.
//   - get(k) is a READ — many concurrent readers OK.
//   - set(k, v) is a WRITE — exclusive.
//
// Three implementations to compare (at minimum):
//   1. std::sync::RwLock (below — kept COMPLETE as the reference baseline).
//   2. Plain std::sync::Mutex. Sometimes wins when critical sections are tiny
//      — RwLock has more atomics per call, overhead is real.
//   3. Hand-rolled lightswitch + turnstile using tokio::sync::Semaphore.
//      tokio's Semaphore is explicitly FIFO — this is the test of whether
//      the C++ std::counting_semaphore fairness gap was the cause of the
//      flakiness in the C++ implementation.
//
// Invariants (enforced by Counters): exactly one of
//   - writers == 0 && readers >= 0   (readers can coexist)
//   - writers == 1 && readers == 0   (writer exclusive)
//
// === Tokio permit model — read this before writing KVCacheHandRolled ===
//
// tokio::sync::Semaphore::acquire() returns a Future<SemaphorePermit<'_>>.
// The permit RELEASES automatically when dropped (RAII).
//
// For the writer, RAII works perfectly — the permits drop at end of scope
// in reverse declaration order (LIFO), giving you free reverse-acquisition
// release order.
//
// For the reader, the "first reader acquires room_empty, last reader
// releases it" pattern crosses *task* boundaries — one task acquires, a
// different task releases. RAII alone can't express that. The idiom:
//   1. First reader: `permit.forget()` — consumes the permit WITHOUT
//      releasing it on drop.
//   2. Last reader: `sem.add_permits(1)` — manually puts it back.
// That's the "forgotten permit" dance, and it matches the C-style
// acquire/release of a semaphore even in Rust's RAII world.

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;
use tokio::sync::Semaphore;

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

// Trait so the benchmark harness is generic over every impl. Async because
// tokio::sync::Semaphore::acquire is an async fn.
#[async_trait]
pub trait Cache: Send + Sync {
    async fn get(&self, k: &str) -> String;
    async fn set(&self, k: String, v: String);
}

// ==========================================================================
// Impl 1: std::sync::RwLock — COMPLETE. Reference baseline.
// The sync RwLock is fine inside async fn because the critical sections are
// tiny (map lookup/insert) and never held across .await.
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

#[async_trait]
impl Cache for KVCacheRw {
    async fn get(&self, k: &str) -> String {
        let guard = self.map.read().unwrap();
        self.counters.enter_read();
        let out = guard.get(k).cloned().unwrap_or_default();
        self.counters.exit_read();
        out
    }
    async fn set(&self, k: String, v: String) {
        let mut guard = self.map.write().unwrap();
        self.counters.enter_write();
        guard.insert(k, v);
        self.counters.exit_write();
    }
}

// ==========================================================================
// Impl 2: hand-rolled lightswitch + turnstile. TODO.
//
// Three Semaphores + an AtomicI32, same shape as your C++/Go turnstile:
//   - turnstile : cap 1. Writers hold for the whole CS; readers gate-pass.
//   - room_empty: cap 1. Held while any reader-collective or writer is in.
//   - bibo      : cap 1. Mutex for rc. All rc manipulation inside.
//   - rc        : AtomicI32. Plain counter; atomic only to satisfy Sync.
//                 Safe as plain-value access because bibo serializes.
// ==========================================================================
pub struct KVCacheHandRolled {
    map: Mutex<HashMap<String, String>>,
    counters: Counters,
    turnstile: Semaphore,
    room_empty: Semaphore,
    bibo: Semaphore,
    rc: AtomicI32,
}

impl KVCacheHandRolled {
    pub fn new() -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
            counters: Counters::new(),
            turnstile: Semaphore::new(1),
            room_empty: Semaphore::new(1),
            bibo: Semaphore::new(1),
            rc: AtomicI32::new(0),
        }
    }
}

#[async_trait]
impl Cache for KVCacheHandRolled {
    async fn get(&self, _k: &str) -> String {
        // TODO: gate-pass the turnstile (acquire then drop immediately).
        //
        // TODO: entry under bibo:
        //   - acquire bibo
        //   - fetch_add(1) on rc; if the NEW value is 1, first reader →
        //     acquire room_empty and `permit.forget()` the permit
        //   - drop bibo (end of block)
        //
        // TODO: critical section:
        //   - self.counters.enter_read()
        //   - lock map (std::sync::Mutex, tight scope, NOT across .await)
        //   - look up key
        //   - self.counters.exit_read()
        //
        // TODO: exit under bibo:
        //   - acquire bibo
        //   - fetch_sub(1) on rc; if the NEW value is 0, last reader →
        //     self.room_empty.add_permits(1) to release the forgotten permit
        //   - drop bibo
        //
        // Skeleton so the file compiles — remove once implemented:
        drop(self.turnstile.acquire().await.unwrap());
        {
            let _bibo = self.bibo.acquire().await.unwrap();
            let _new_rc = self.rc.fetch_add(1, Ordering::SeqCst) + 1;
            if _new_rc == 1 {
            //wtf! you can do this!
            // ?? Permit dies here what can we do???
            let permit = self.room_empty.acquire().await.unwrap();
            permit.forget();
            }
        }


        self.counters.enter_read();
        let out = {
            let guard = self.map.lock().unwrap();
            guard.get(_k).cloned().unwrap_or_default()
        };
        self.counters.exit_read();
        {
            let _bibo = self.bibo.acquire().await.unwrap();
            let _new_rc = self.rc.fetch_sub(1, Ordering::SeqCst) - 1;
            if _new_rc == 0 {
                self.room_empty.add_permits(1);
            }
        }
        out
    }

    async fn set(&self, _k: String, _v: String) {
        // TODO: writer holds turnstile for the whole CS.
        //   - let _turnstile = self.turnstile.acquire().await.unwrap();
        //   - let _room      = self.room_empty.acquire().await.unwrap();
        //   - self.counters.enter_write()
        //   - lock map in a tight scope and insert
        //   - self.counters.exit_write()
        //   - permits drop at end of fn in reverse declaration order (LIFO):
        //     _room drops first, then _turnstile. That's the correct
        //     reverse-acquisition release order.
        //
        // Skeleton so the file compiles — remove once implemented:
        let _turnstile = self.turnstile.acquire().await.unwrap();
        let _room = self.room_empty.acquire().await.unwrap();
        self.counters.enter_write();
        let mut guard = self.map.lock().unwrap();
        guard.insert(_k, _v);
        drop(guard);
        self.counters.exit_write();
    }
}

// --------------------------------------------------------------------------
// Generic benchmark harness — times reads + writes across tokio tasks.
// --------------------------------------------------------------------------
async fn run_benchmark<C: Cache + 'static>(name: &str, cache: Arc<C>) {
    let start = Instant::now();
    let mut handles = Vec::new();

    for r in 0..NUM_READERS {
        let c = Arc::clone(&cache);
        handles.push(tokio::spawn(async move {
            for i in 0..OPS_PER_THREAD {
                let _ = c.get(&format!("key{}", (r + i) % 10)).await;
            }
        }));
    }
    for w in 0..NUM_WRITERS {
        let c = Arc::clone(&cache);
        handles.push(tokio::spawn(async move {
            for i in 0..OPS_PER_THREAD {
                c.set(format!("key{}", i % 10), format!("v{}", i)).await;
            }
            let _ = w;
        }));
    }
    for h in handles {
        h.await.unwrap();
    }
    println!("{:<14} {} ms", name, start.elapsed().as_millis());
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    run_benchmark("RwLock", Arc::new(KVCacheRw::new())).await;

    // TODO: once KVCacheHandRolled actually locks, uncomment:
    run_benchmark("HandRolled", Arc::new(KVCacheHandRolled::new())).await;
}
