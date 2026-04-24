// Producer-Consumer — starter template (Rust)
//
// Scenario: log aggregator. NUM_PRODUCERS → bounded buffer → NUM_CONSUMERS.
//
// Three implementations to try:
//   1. Mutex<VecDeque> + Condvar (shown as skeleton below).
//   2. std::sync::mpsc::sync_channel(capacity) — the "easy" Rust way.
//         MPSC = Multi-Producer, Single-Consumer; the std channel is MPSC only.
//         For multi-consumer, use `crossbeam-channel` or share a single receiver
//         behind a Mutex.
//   3. lock-free ringbuffer (stretch; `crossbeam-queue`).
//
// Classic bugs to check for:
//   - Forgetting `while` (predicate loop) around cv.wait — spurious wakeups.
//   - Shutdown hangs: consumers stuck waiting after producers exit.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

const BUFFER_SIZE: usize = 8;
const NUM_PRODUCERS: usize = 3;
const NUM_CONSUMERS: usize = 2;
const ITEMS_PER_PRODUCER: i32 = 50;

pub struct BoundedBuffer<T> {
    inner: Mutex<BufferInner<T>>,
    not_full: Condvar,
    not_empty: Condvar,
    capacity: usize,
}

struct BufferInner<T> {
    queue: VecDeque<T>,
    closed: bool,
}

impl<T> BoundedBuffer<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(BufferInner {
                queue: VecDeque::with_capacity(capacity),
                closed: false,
            }),
            not_full: Condvar::new(),
            not_empty: Condvar::new(),
            capacity,
        }
    }

    /// Returns Err(item) if buffer was closed before push completed.
    pub fn push(&self, item: T) -> Result<(), T> {
        // ================================================================
        // TODO:
        //   - lock inner
        //   - while queue.len() == capacity && !closed:
        //         inner = self.not_full.wait(inner).unwrap()
        //   - if closed: return Err(item)
        //   - queue.push_back(item)
        //   - notify_one on not_empty
        //   - Ok(())
        // ================================================================
        let mut inner = self.inner.lock().unwrap();
        while !(inner.queue.len() < self.capacity || inner.closed) {
            inner = self.not_full.wait(inner).unwrap();
        }
        if inner.closed {
            Err(item)
        }
        else {
            inner.queue.push_back(item);
            self.not_empty.notify_one();
            Ok(())
        }

    }

    /// Returns None when closed AND empty — signals consumers to exit.
    pub fn pop(&self) -> Option<T> {
        // ================================================================
        // TODO:
        //   - lock inner
        //   - while queue is empty && !closed:
        //         inner = self.not_empty.wait(inner).unwrap()
        //   - if queue is empty && closed: return None
        //   - let item = queue.pop_front().unwrap()
        //   - notify_one on not_full
        //   - Some(item)
        // ================================================================
        let mut inner = self.inner.lock().unwrap();
        while inner.queue.is_empty() && !inner.closed {
            inner = self.not_empty.wait(inner).unwrap();
        }
        if inner.closed && inner.queue.is_empty() {
            None
        }
        else {
            //  inner.queue.pop_front() already returns Option<T>, which matches the function's return type, so item works. But   
            // the TODO suggests let item = queue.pop_front().unwrap(); ... Some(item). That .unwrap() is a self-check: "by my own invariant, the queue 
            // can't be empty at this point — crash if I'm wrong." Worth doing because if your wait predicate ever drifts, you'll find out loudly instead  
            // of silently returning None. Optional.

            let item = inner.queue.pop_front().unwrap();
            self.not_full.notify_one();
            Some(item)
        }
    }

    pub fn close(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.closed = true;
        self.not_empty.notify_all();
        self.not_full.notify_all();
    }
}

fn main() {
    let buffer = Arc::new(BoundedBuffer::<String>::new(BUFFER_SIZE));
    let produced = Arc::new(AtomicI32::new(0));
    let consumed = Arc::new(AtomicI32::new(0));
    let producers_done = Arc::new(AtomicI32::new(0));

    let mut producers = Vec::new();
    for pid in 0..NUM_PRODUCERS {
        let buffer = Arc::clone(&buffer);
        let produced = Arc::clone(&produced);
        let producers_done = Arc::clone(&producers_done);
        producers.push(thread::spawn(move || {
            for i in 0..ITEMS_PER_PRODUCER {
                let item = format!("P{}#{}", pid, i);
                if buffer.push(item).is_err() {
                    break;
                }
                produced.fetch_add(1, Ordering::SeqCst);
                thread::sleep(Duration::from_millis((pid as u64) % 3));
            }
            if producers_done.fetch_add(1, Ordering::SeqCst) + 1
                == NUM_PRODUCERS as i32
            {
                buffer.close();
            }
        }));
    }

    let mut consumers = Vec::new();
    for _cid in 0..NUM_CONSUMERS {
        let buffer = Arc::clone(&buffer);
        let consumed = Arc::clone(&consumed);
        consumers.push(thread::spawn(move || {
            while let Some(item) = buffer.pop() {
                std::hint::black_box(item);
                consumed.fetch_add(1, Ordering::SeqCst);
            }
        }));
    }

    for p in producers {
        p.join().unwrap();
    }
    for c in consumers {
        c.join().unwrap();
    }

    let expected = (NUM_PRODUCERS as i32) * ITEMS_PER_PRODUCER;
    let p = produced.load(Ordering::SeqCst);
    let c = consumed.load(Ordering::SeqCst);
    println!("Produced={} Consumed={} Expected={}", p, c, expected);
    assert_eq!(p, expected, "not all items produced");
    assert_eq!(c, expected, "not all items consumed");
}
