// Dining Philosophers — starter template (Rust)
//
// Your job: fill in the `eat` method using your chosen strategy.
// Rust's Send/Sync checks will stop you from sharing Mutexes incorrectly,
// but logic bugs (deadlock, starvation, livelock) are still all yours.
//
// Strategies worth implementing (try at least two):
//   1. Naive Arc<Mutex> per chopstick — will deadlock. PROVE IT.
//   2. Asymmetric ordering: philosopher N-1 grabs right first.
//   3. try_lock with backoff (can livelock — demonstrate it).
//   4. Channel-based (mpsc) — send chopstick "tokens" around.
//   5. tokio async version (stretch).
//
// Use the invariant helper inside the critical section.

use rand::Rng;
use std::sync::atomic::{AtomicI32, AtomicU8, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const N: usize = 5;
const MEALS_PER_PHILOSOPHER: i32 = 50;

// State as u8 so we can put it in AtomicU8.
const THINKING: u8 = 0;
const HUNGRY: u8 = 1;
const EATING: u8 = 2;

struct Shared {
    states: [AtomicU8; N],
    meals: [AtomicI32; N],
    // TODO: add your chopsticks/semaphores/channels here.
    // Example: chopsticks: [Mutex<()>; N],
}

impl Shared {
    fn new() -> Self {
        Self {
            states: std::array::from_fn(|_| AtomicU8::new(THINKING)),
            meals: std::array::from_fn(|_| AtomicI32::new(0)),
        }
    }

    fn check_invariant(&self, pid: usize) {
        let left = (pid + N - 1) % N;
        let right = (pid + 1) % N;
        assert_ne!(
            self.states[left].load(Ordering::SeqCst),
            EATING,
            "philosopher {pid} eating while left {left} is eating"
        );
        assert_ne!(
            self.states[right].load(Ordering::SeqCst),
            EATING,
            "philosopher {pid} eating while right {right} is eating"
        );
    }
}

fn jitter(max_ms: u64) {
    let ms = rand::thread_rng().gen_range(0..=max_ms);
    thread::sleep(Duration::from_millis(ms));
}

fn think(shared: &Shared, pid: usize) {
    shared.states[pid].store(THINKING, Ordering::SeqCst);
    jitter(3);
}

fn eat(shared: &Shared, pid: usize) {
    shared.states[pid].store(HUNGRY, Ordering::SeqCst);

    // ======================================================================
    // TODO: Acquire chopsticks / resources here.
    // ======================================================================

    shared.states[pid].store(EATING, Ordering::SeqCst);
    shared.check_invariant(pid);
    shared.meals[pid].fetch_add(1, Ordering::SeqCst);
    jitter(3);
    shared.states[pid].store(THINKING, Ordering::SeqCst);

    // ======================================================================
    // TODO: Release.
    // ======================================================================
}

fn philosopher(shared: Arc<Shared>, pid: usize) {
    for _ in 0..MEALS_PER_PHILOSOPHER {
        think(&shared, pid);
        eat(&shared, pid);
    }
}

fn main() {
    let shared = Arc::new(Shared::new());
    let start = Instant::now();

    let handles: Vec<_> = (0..N)
        .map(|pid| {
            let s = Arc::clone(&shared);
            thread::spawn(move || philosopher(s, pid))
        })
        .collect();

    for h in handles {
        h.join().unwrap();
    }

    let elapsed = start.elapsed();
    println!("Done in {} ms", elapsed.as_millis());

    let meals: Vec<i32> = shared
        .meals
        .iter()
        .map(|m| m.load(Ordering::SeqCst))
        .collect();
    println!("Meals per philosopher: {meals:?}");
    let min = *meals.iter().min().unwrap();
    let max = *meals.iter().max().unwrap();
    println!("min={min} max={max} spread={}", max - min);
}
