# Sketches — Reference Shapes for the Templates Not Yet Written

These sketches mirror the patterns already in the completed C++ templates and the existing `go/dining_philosophers/main.go`. They're **reference text**, not complete files — use them as the "shape" when you create the actual files yourself.

Every template follows the same skeleton:
1. Constants at the top (sizes, counts).
2. A shared state struct with atomics for invariant-check counters.
3. An invariant-checker method that `panic`s on violation.
4. A `jitter()` helper to expose races.
5. `TODO` blocks where you fill in the sync logic.
6. `main()` that spawns, joins, and reports fairness / correctness stats.

---

## Go: `barrier/main.go`

```go
package main

import (
	"fmt"
	"sync"
	"sync/atomic"
	"time"
)

const (
	NThreads = 8
	Rounds   = 100
)

// ==========================================================================
// TODO: Replace with YOUR barrier implementation. Try at least two:
//   1. sync.Mutex + sync.Cond + count + generation counter.
//   2. Two sync.WaitGroups (lecture slide 23 pattern).
//   3. Channel-based: `chan struct{}` that the last arriver closes,
//      then reopens for next round (tricky — watch for races).
//   4. (Compare) golang.org/x/sync/errgroup or just call sync.WaitGroup
//      if you only need single-use.
// ==========================================================================
type MyBarrier struct {
	expected   int
	count      int
	generation int
	mu         sync.Mutex
	cv         *sync.Cond
}

func NewMyBarrier(expected int) *MyBarrier {
	b := &MyBarrier{expected: expected}
	b.cv = sync.NewCond(&b.mu)
	return b
}

func (b *MyBarrier) ArriveAndWait() {
	// TODO:
	//   - lock
	//   - gen := b.generation
	//   - b.count++
	//   - if b.count == b.expected:
	//         b.count = 0
	//         b.generation++
	//         b.cv.Broadcast()
	//   - else: for b.generation == gen { b.cv.Wait() }
	//   - unlock
}

var (
	globalRound       atomic.Int32
	arrivedThisRound  atomic.Int32
)

func worker(tid int, barrier *MyBarrier, wg *sync.WaitGroup) {
	defer wg.Done()
	for r := 0; r < Rounds; r++ {
		// pretend work
		var x uint64
		for i := uint64(0); i < 1000; i++ {
			x += i
		}
		_ = x

		barrier.ArriveAndWait()

		seen := globalRound.Load()
		if seen != int32(r) {
			panic(fmt.Sprintf("thread %d saw round %d but expected %d", tid, seen, r))
		}
		if arrivedThisRound.Add(1) == int32(NThreads) {
			arrivedThisRound.Store(0)
			globalRound.Store(int32(r + 1))
		}

		barrier.ArriveAndWait()
	}
}

func main() {
	b := NewMyBarrier(NThreads)
	var wg sync.WaitGroup
	start := time.Now()
	for t := 0; t < NThreads; t++ {
		wg.Add(1)
		go worker(t, b, &wg)
	}
	wg.Wait()
	fmt.Printf("Completed %d rounds with %d threads in %v\n",
		Rounds, NThreads, time.Since(start))
}
```

**Run:** `go run -race .` — the race detector will catch most mistakes in the `Wait`/`Broadcast` dance.

---

## Go: `barbershop/main.go`

Go shines on this one — channels are a natural fit. The first implementation should use channels; save the mutex+cond version for second.

```go
package main

import (
	"fmt"
	"math/rand"
	"sync"
	"sync/atomic"
	"time"
)

const (
	Chairs         = 3
	TotalCustomers = 30
)

// ==========================================================================
// Implementation sketch — channel-based (first attempt).
// Trade-offs vs semaphore version:
//   + Very clean. chairs channel naturally bounds waiters.
//   - FIFO not guaranteed (Go's select is randomized).
//   - Harder to cleanly express "customer waits for THIS barber to
//     finish MY haircut" — customers might get served in any order.
// For strict FIFO, second implementation should use a mutex+queue.
// ==========================================================================
type Barbershop struct {
	chairs         chan int        // buffered; slot taken by customer ID
	barberReady    chan struct{}   // unbuffered; rendezvous
	customerDone   chan struct{}
	barberDone     chan struct{}
	shutdown       chan struct{}
}

func NewBarbershop() *Barbershop {
	return &Barbershop{
		chairs:       make(chan int, Chairs),
		barberReady:  make(chan struct{}),
		customerDone: make(chan struct{}),
		barberDone:   make(chan struct{}),
		shutdown:     make(chan struct{}),
	}
}

// Returns true if served, false if balked.
func (bs *Barbershop) Customer(id int) bool {
	// TODO:
	//   select {
	//   case bs.chairs <- id:   // sat down
	//       // wait for barber to be ready
	//       <-bs.barberReady
	//       // get haircut (sleep)
	//       time.Sleep(...)
	//       // tell barber I'm done
	//       bs.customerDone <- struct{}{}
	//       // wait for barber's final ack
	//       <-bs.barberDone
	//       // leave seat — BUT: when do we drain `chairs`?
	//       //    One trick: barber receives from `chairs` to call next customer.
	//       return true
	//   default:                // shop full — balk
	//       return false
	//   }
	return false
}

func (bs *Barbershop) Barber() {
	for {
		select {
		case <-bs.shutdown:
			return
		case id := <-bs.chairs:
			_ = id
			// TODO:
			//   - send on bs.barberReady (customer starts haircut)
			//   - "cut hair" (sleep)
			//   - receive on bs.customerDone
			//   - send on bs.barberDone
		}
	}
}

func (bs *Barbershop) Shutdown() { close(bs.shutdown) }

func main() {
	shop := NewBarbershop()
	go shop.Barber()

	var served, balked atomic.Int32
	var wg sync.WaitGroup

	for i := 0; i < TotalCustomers; i++ {
		time.Sleep(time.Duration(rand.Intn(30)) * time.Millisecond)
		wg.Add(1)
		go func(id int) {
			defer wg.Done()
			if shop.Customer(id) {
				served.Add(1)
			} else {
				balked.Add(1)
			}
		}(i)
	}
	wg.Wait()
	shop.Shutdown()

	total := served.Load() + balked.Load()
	fmt.Printf("Served=%d Balked=%d Total=%d (expected %d)\n",
		served.Load(), balked.Load(), total, TotalCustomers)
	if total != TotalCustomers {
		panic("lost customers")
	}
}
```

**Common gotcha:** the chair slot — who clears it? Pattern I use: the *barber* receives from `chairs` (that's what "calls the next customer"). Don't try to clear it from the customer side; you'll get ordering bugs.

---

## Go: `producer_consumer/main.go`

In Go, **the bounded buffer is literally just a buffered channel**. So this template has to be framed as "don't use a channel" for the first implementation — otherwise you're not learning anything. Two versions:

```go
// ============================================================
// Version 1: WITHOUT channels. Mutex + sync.Cond.
// Purpose: force you to think through the condvar dance that
// languages without channels require. This is what C++ / Rust
// / Java developers write.
// ============================================================
package main

import (
	"container/list"
	"fmt"
	"sync"
	"sync/atomic"
	"time"
)

const (
	BufferSize        = 8
	NumProducers      = 3
	NumConsumers      = 2
	ItemsPerProducer  = 50
)

type BoundedBuffer struct {
	mu       sync.Mutex
	notFull  *sync.Cond
	notEmpty *sync.Cond
	q        *list.List
	capacity int
	closed   bool
}

func NewBoundedBuffer(cap int) *BoundedBuffer {
	b := &BoundedBuffer{q: list.New(), capacity: cap}
	b.notFull = sync.NewCond(&b.mu)
	b.notEmpty = sync.NewCond(&b.mu)
	return b
}

// Returns false if buffer was closed.
func (b *BoundedBuffer) Push(item string) bool {
	// TODO:
	//   b.mu.Lock(); defer b.mu.Unlock()
	//   for b.q.Len() == b.capacity && !b.closed { b.notFull.Wait() }
	//   if b.closed { return false }
	//   b.q.PushBack(item)
	//   b.notEmpty.Signal()
	//   return true
	return false
}

// Returns ("", false) if closed AND empty.
func (b *BoundedBuffer) Pop() (string, bool) {
	// TODO mirror image
	return "", false
}

func (b *BoundedBuffer) Close() {
	b.mu.Lock()
	defer b.mu.Unlock()
	b.closed = true
	b.notFull.Broadcast()
	b.notEmpty.Broadcast()
}

func main() {
	buf := NewBoundedBuffer(BufferSize)
	var produced, consumed, producersDone atomic.Int32
	var pwg, cwg sync.WaitGroup

	for p := 0; p < NumProducers; p++ {
		pwg.Add(1)
		go func(pid int) {
			defer pwg.Done()
			for i := 0; i < ItemsPerProducer; i++ {
				if !buf.Push(fmt.Sprintf("P%d#%d", pid, i)) {
					return
				}
				produced.Add(1)
				time.Sleep(time.Duration(pid%3) * time.Millisecond)
			}
			if producersDone.Add(1) == int32(NumProducers) {
				buf.Close()
			}
		}(p)
	}

	for c := 0; c < NumConsumers; c++ {
		cwg.Add(1)
		go func() {
			defer cwg.Done()
			for {
				if _, ok := buf.Pop(); !ok {
					return
				}
				consumed.Add(1)
			}
		}()
	}
	pwg.Wait()
	cwg.Wait()

	expected := int32(NumProducers * ItemsPerProducer)
	fmt.Printf("Produced=%d Consumed=%d Expected=%d\n",
		produced.Load(), consumed.Load(), expected)
}
```

```go
// ============================================================
// Version 2: idiomatic Go — just use a buffered channel.
// Purpose: appreciate how much ceremony channels eliminate.
// ============================================================
// items := make(chan string, BufferSize)  // that's your bounded buffer
//
// Producer: items <- item
// Consumer: for item := range items { ... }
// Shutdown: close(items) (only the LAST producer should close)
//
// The whole file shrinks to ~30 lines. Compare LoC side-by-side.
```

The learning happens when you've written both and can articulate: *when* does the channel version feel cleaner, and *when* does it hide a semantic distinction you actually needed?

---

## Go: `readers_writers/main.go`

Go has `sync.RWMutex` in the standard library. For this problem the learning is **not** "implement a lock from scratch" but rather "understand what sync.RWMutex gives you and what it doesn't."

```go
package main

import (
	"fmt"
	"sync"
	"sync/atomic"
	"time"
)

const (
	NumReaders    = 8
	NumWriters    = 2
	OpsPerThread  = 1000
)

// ==========================================================================
// Invariant counters — wrap every critical section.
// ==========================================================================
type Counters struct {
	readers atomic.Int32
	writers atomic.Int32
}

func (c *Counters) EnterRead() {
	c.readers.Add(1)
	if c.writers.Load() != 0 {
		panic("reader entered with writers active")
	}
}
func (c *Counters) ExitRead() { c.readers.Add(-1) }

func (c *Counters) EnterWrite() {
	c.writers.Add(1)
	if c.writers.Load() != 1 || c.readers.Load() != 0 {
		panic(fmt.Sprintf("writer entered with writers=%d readers=%d",
			c.writers.Load(), c.readers.Load()))
	}
}
func (c *Counters) ExitWrite() { c.writers.Add(-1) }

// ==========================================================================
// Impl 1: sync.RWMutex (library)
// ==========================================================================
type KVCacheRW struct {
	mu       sync.RWMutex
	data     map[string]string
	counters Counters
}

func NewKVCacheRW() *KVCacheRW {
	return &KVCacheRW{data: make(map[string]string)}
}

func (c *KVCacheRW) Get(k string) string {
	c.mu.RLock()
	defer c.mu.RUnlock()
	c.counters.EnterRead()
	defer c.counters.ExitRead()
	return c.data[k]
}

func (c *KVCacheRW) Set(k, v string) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.counters.EnterWrite()
	defer c.counters.ExitWrite()
	c.data[k] = v
}

// ==========================================================================
// Impl 2: plain sync.Mutex — compare throughput. Sometimes the simpler
// lock wins when critical sections are tiny.
// ==========================================================================
type KVCacheMu struct {
	mu       sync.Mutex
	data     map[string]string
	counters Counters
}

// TODO: implement Get/Set with plain Mutex.

// ==========================================================================
// Impl 3: hand-rolled lightswitch pattern (lecture slide 10-11).
// Use this to reproduce writer starvation, then fix with a turnstile.
// ==========================================================================
// TODO.

func main() {
	// Benchmark runs for each impl. Keep it simple: just time it.
	bench := func(name string, get func(string) string, set func(string, string)) {
		var wg sync.WaitGroup
		start := time.Now()
		for r := 0; r < NumReaders; r++ {
			wg.Add(1)
			go func(rid int) {
				defer wg.Done()
				for i := 0; i < OpsPerThread; i++ {
					get(fmt.Sprintf("key%d", i%10))
				}
			}(r)
		}
		for w := 0; w < NumWriters; w++ {
			wg.Add(1)
			go func(wid int) {
				defer wg.Done()
				for i := 0; i < OpsPerThread; i++ {
					set(fmt.Sprintf("key%d", i%10), fmt.Sprintf("v%d", i))
				}
			}(w)
		}
		wg.Wait()
		fmt.Printf("%-14s %v\n", name, time.Since(start))
	}

	rw := NewKVCacheRW()
	bench("RWMutex", rw.Get, rw.Set)

	// TODO: bench("Mutex", ...) and bench("handrolled", ...)
}
```

**Measurement question to answer:** at what read:write ratio does `sync.RWMutex` actually beat plain `sync.Mutex`? (Empirically it's often higher than you'd guess — the RWMutex has more atomic operations per call.)

---

## Rust: `readers_writers/`

Shape is very close to the C++ version. Key primitive: `std::sync::RwLock<HashMap<String, String>>`.

### `rust/readers_writers/Cargo.toml`
```toml
[package]
name = "readers_writers"
version = "0.1.0"
edition.workspace = true

[dependencies]
rand = "0.8"
```

### `rust/readers_writers/src/main.rs` sketch
```rust
use std::collections::HashMap;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::Instant;

const NUM_READERS: usize = 8;
const NUM_WRITERS: usize = 2;
const OPS_PER_THREAD: usize = 1000;

struct Counters {
    readers: AtomicI32,
    writers: AtomicI32,
}
impl Counters {
    fn new() -> Self {
        Self { readers: AtomicI32::new(0), writers: AtomicI32::new(0) }
    }
    fn enter_read(&self) {
        self.readers.fetch_add(1, Ordering::SeqCst);
        assert_eq!(self.writers.load(Ordering::SeqCst), 0,
                   "reader entered with writers active");
    }
    fn exit_read(&self) { self.readers.fetch_sub(1, Ordering::SeqCst); }
    fn enter_write(&self) {
        self.writers.fetch_add(1, Ordering::SeqCst);
        let w = self.writers.load(Ordering::SeqCst);
        let r = self.readers.load(Ordering::SeqCst);
        assert!(w == 1 && r == 0,
                "writer entered with writers={w} readers={r}");
    }
    fn exit_write(&self) { self.writers.fetch_sub(1, Ordering::SeqCst); }
}

// Impl 1: stdlib RwLock
struct KVCacheRw {
    map: RwLock<HashMap<String, String>>,
    counters: Counters,
}
impl KVCacheRw {
    fn new() -> Self {
        Self { map: RwLock::new(HashMap::new()), counters: Counters::new() }
    }
    fn get(&self, k: &str) -> Option<String> {
        let guard = self.map.read().unwrap();
        self.counters.enter_read();
        let result = guard.get(k).cloned();
        self.counters.exit_read();
        result
    }
    fn set(&self, k: String, v: String) {
        let mut guard = self.map.write().unwrap();
        self.counters.enter_write();
        guard.insert(k, v);
        self.counters.exit_write();
    }
}

// Impl 2: Mutex (plain) — TODO, for comparison.
// Impl 3: hand-rolled with Mutex<(count, writer_active)> + Condvar.
//         This is where you can reproduce writer starvation, then
//         implement the turnstile fix from lecture slide 12.

fn main() {
    // mirror the C++ run_benchmark — spawn readers + writers, time it.
    let cache = Arc::new(KVCacheRw::new());
    let start = Instant::now();
    let mut handles = vec![];

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
    for h in handles { h.join().unwrap(); }

    println!("RwLock: {} ms", start.elapsed().as_millis());
}
```

Then uncomment the `readers_writers` line in `rust/Cargo.toml` workspace members.

---

## Commonalities worth noticing as you write these

1. **C++ has semaphores** (`std::counting_semaphore` since C++20), **Rust does not** (build from Mutex+Condvar, or use `tokio::sync::Semaphore` or `crossbeam`). **Go has neither** stdlib semaphores — you emulate with a buffered channel of the right capacity.

2. **Rust's `Condvar::wait` returns `LockResult<MutexGuard>`**, which you unwrap. Go's `sync.Cond.Wait()` implicitly releases and re-acquires the lock. C++ takes the lock by reference. Three languages, three ergonomic shapes for the same concept.

3. **Go's `-race` flag** is the best-in-class race detector here. Use it religiously. Rust gets you much of the same safety *at compile time* but not for logic errors. C++ has nothing by default — you have to opt in to TSan.

4. **Channels force a different decomposition.** When you translate a mutex-based solution to channels (or vice versa), you'll often realize halfway through that one style makes a property trivial (say, FIFO for a mutex + explicit queue) that's painful in the other (FIFO with channels + `select` is surprisingly tricky because `select` cases are randomized).

Those three insights are most of what you're here to learn.
