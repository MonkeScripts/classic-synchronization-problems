// Readers-Writers — starter template (Go)
//
// Scenario: in-memory KV cache.
//   - Get(k) is a READ — many concurrent readers OK.
//   - Set(k, v) is a WRITE — exclusive.
//
// Three implementations to compare (at minimum):
//   1. sync.RWMutex (below — kept complete as the reference baseline).
//   2. Plain sync.Mutex. Sometimes wins when critical sections are tiny —
//      RWMutex has more atomic ops per call and that overhead is real.
//   3. Hand-rolled "lightswitch" (lecture slide 10-11) with Mutex+Cond.
//      Reproduce writer starvation, then fix it with the turnstile pattern
//      from slide 12.
//
// Invariants (checked by Counters below, wrap every critical section):
//   - active_writers <= 1
//   - active_readers > 0  =>  active_writers == 0
//   - active_writers == 1 =>  active_readers == 0
//
// Measurement question (answer by running):
//   At what read:write ratio does sync.RWMutex actually beat sync.Mutex?
//   (Empirically higher than you'd guess.)

package main

import (
	"fmt"
	"sync"
	"sync/atomic"
	"time"
)

const (
	NumReaders   = 8
	NumWriters   = 2
	OpsPerThread = 1000
)

// --------------------------------------------------------------------------
// Runtime invariant tracker. Same shape as the C++/Rust templates.
// --------------------------------------------------------------------------
type Counters struct {
	readers atomic.Int32
	writers atomic.Int32
}

func (c *Counters) EnterRead() {
	c.readers.Add(1)
	if c.writers.Load() != 0 {
		panic("reader entered while writer active")
	}
}
func (c *Counters) ExitRead() { c.readers.Add(-1) }

func (c *Counters) EnterWrite() {
	c.writers.Add(1)
	if c.writers.Load() != 1 || c.readers.Load() != 0 {
		panic(fmt.Sprintf(
			"writer entered with writers=%d readers=%d",
			c.writers.Load(), c.readers.Load()))
	}
}
func (c *Counters) ExitWrite() { c.writers.Add(-1) }

// Cache is the surface every impl provides. The benchmark is generic over it.
type Cache interface {
	Get(k string) string
	Set(k, v string)
}

// ==========================================================================
// Impl 1: sync.RWMutex — COMPLETE. Use as the reference baseline.
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
// Impl 2: hand-rolled lightswitch. TODO.
//
// Fields you probably want (uncomment/add as you design):
//   mu             sync.Mutex       // protects `readersIn` + condvar
//   readersIn      int              // number of readers currently in CS
//   roomEmpty      *sync.Cond       // writers wait on this
//   (and a turnstile semaphore-ish thing if you want the no-starve variant)
// ==========================================================================
type KVCacheHandRolled struct {
	data     map[string]string
	counters Counters
	bibo chan struct{}
	emptyRoom chan struct{}
	turnstile chan struct{}
	rc int


	// TODO: add your lock fields (mu, cond, counter, optional turnstile).
}

func NewKVCacheHandRolled() *KVCacheHandRolled {
	return &KVCacheHandRolled{
		data: make(map[string]string),
		bibo: make(chan struct{}, 1),
		emptyRoom: make(chan struct{}, 1),
		turnstile: make(chan struct{}, 1),
	}
}

func (c *KVCacheHandRolled) Get(k string) string {
	// TODO: "read lock" — first reader must block new writers; other readers
	// pile on. Slide 10 lightswitch.lock(roomEmpty) pattern.
	c.turnstile <- struct{}{}
	<-c.turnstile
	c.bibo <- struct{}{}
	c.rc++
	if c.rc == 1 {
		c.emptyRoom <- struct{}{}
	}
	<-c.bibo
	c.counters.EnterRead()
	v := c.data[k]
	c.counters.ExitRead()
	c.bibo <- struct{}{}
	c.rc--
	if c.rc == 0 {
		<-c.emptyRoom
	}
	<-c.bibo

	// TODO: "read unlock" — last reader releases the writer block.
	return v
}

func (c *KVCacheHandRolled) Set(k, v string) {
	// TODO: wait until roomEmpty, then take it exclusively.
	// For the no-starve variant (slide 12): acquire a turnstile first so
	// new readers queue up behind any waiting writer, then take roomEmpty.
	c.turnstile <- struct{}{}
	c.emptyRoom <- struct{}{}
	c.counters.EnterWrite()
	c.data[k] = v
	c.counters.ExitWrite()
	<-c.emptyRoom
	<-c.turnstile


	// TODO: release.
}

// --------------------------------------------------------------------------
// Benchmark harness — generic over any Cache.
// --------------------------------------------------------------------------
func bench(name string, cache Cache) {
	var wg sync.WaitGroup
	start := time.Now()

	for r := 0; r < NumReaders; r++ {
		wg.Add(1)
		go func(rid int) {
			defer wg.Done()
			for i := 0; i < OpsPerThread; i++ {
				_ = cache.Get(fmt.Sprintf("key%d", (rid+i)%10))
			}
		}(r)
	}
	for w := 0; w < NumWriters; w++ {
		wg.Add(1)
		go func(wid int) {
			defer wg.Done()
			for i := 0; i < OpsPerThread; i++ {
				cache.Set(fmt.Sprintf("key%d", i%10), fmt.Sprintf("v%d", i))
			}
			_ = wid
		}(w)
	}
	wg.Wait()
	fmt.Printf("%-14s %v\n", name, time.Since(start))
}

func main() {
	bench("RWMutex", NewKVCacheRW())

	// TODO: once you've implemented KVCacheHandRolled, uncomment:
	bench("HandRolled", NewKVCacheHandRolled())
}
