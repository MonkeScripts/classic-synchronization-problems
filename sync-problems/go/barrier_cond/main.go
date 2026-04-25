// Barrier — mutex + sync.Cond + generation counter (Go).
//
// Sibling to ./barrier (which uses the two-WaitGroup slide-23 shape).
// This is the "classic" reusable barrier:
//   - one mutex
//   - one *sync.Cond
//   - one count (current arrivals)
//   - one generation counter (bumped each time the barrier trips)
//
// Each thread snapshots `gen` on entry and waits while `gen == b.generation`.
// The last arriver resets count, bumps generation, and Broadcasts. Spurious
// wakeups are handled by the predicate-loop, which is the whole point of the
// generation counter (versus a bare boolean flag).
//
// Run during development WITH the race detector:
//     go run -race .

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

type MyBarrier struct {
	mu         sync.Mutex
	cond       *sync.Cond
	expected   int
	count      int    // arrivals in the current generation
	generation uint64 // bumped each time the barrier trips
}

func NewMyBarrier(expected int) *MyBarrier {
	b := &MyBarrier{expected: expected}
	b.cond = sync.NewCond(&b.mu)
	return b
}

func (b *MyBarrier) ArriveAndWait() {
	b.mu.Lock()
	defer b.mu.Unlock()

	gen := b.generation
	b.count++

	if b.count == b.expected {
		// Last one in: trip the barrier and start a new generation.
		b.count = 0
		b.generation++
		b.cond.Broadcast()
		return
	}

	// Wait until the generation advances.
	for gen == b.generation {
		b.cond.Wait()
	}
}

// --------------------------------------------------------------------------
// Invariant tracking — mirrors ./barrier/main.go.
// Any worker that re-enters round R+1 before ALL workers left round R
// will trigger the assert in worker().
// --------------------------------------------------------------------------
var (
	globalRound      atomic.Int32
	arrivedThisRound atomic.Int32
)

func worker(tid int, barrier *MyBarrier, wg *sync.WaitGroup) {
	defer wg.Done()
	for r := 0; r < Rounds; r++ {
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
