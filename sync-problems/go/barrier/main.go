// Barrier — starter template (Go)
//
// Run during development WITH the race detector:
//     go run -race .
//
// Your job: implement a REUSABLE barrier. Try at least two of these:
//   1. sync.Mutex + *sync.Cond + count + generation counter (classic).
//   2. Two sync.WaitGroups (lecture slide 23 — Barrier1 with wg/wg2).
//   3. Channel-close trick: last arriver close()s a chan struct{}, then
//      the barrier swaps in a fresh chan for the next round. Tricky —
//      watch for the "one lap ahead" race when reading the chan field.
//   4. (Compare to) sync.WaitGroup alone — but note it is single-use,
//      which is exactly why the reusable versions above are interesting.
//
// Invariant: when any thread enters round R+1, NO thread is still in round R.
// The global_round / arrived_this_round atomics + assert below enforce this
// across BOTH sides of the barrier. If you get the reset wrong, the assert
// will fire on a later round (not always round 1).

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
// TODO: Replace this placeholder with YOUR barrier implementation.
// The skeleton below picks the mutex+cond shape; feel free to replace the
// whole struct if you pick a different strategy (e.g. channels).
// ==========================================================================
type MyBarrier struct {
	expected int
	// TODO: fields you need — count, generation, mu sync.Mutex, cv *sync.Cond,
	// or a chan struct{}, or two sync.WaitGroups, ...
}

func NewMyBarrier(expected int) *MyBarrier {
	// TODO: initialize whatever fields your chosen strategy needs.
	return &MyBarrier{expected: expected}
}

func (b *MyBarrier) ArriveAndWait() {
	// TODO: block until `expected` goroutines have called ArriveAndWait,
	// then release all of them. MUST be reusable across rounds.
	//
	// Shape for the mutex+cond version:
	//   - lock
	//   - remember gen := b.generation
	//   - b.count++
	//   - if b.count == b.expected:
	//         b.count = 0
	//         b.generation++
	//         b.cv.Broadcast()
	//   - else:
	//         for b.generation == gen { b.cv.Wait() }
	//   - unlock
	_ = b.expected
}

// --------------------------------------------------------------------------
// Invariant tracking — do NOT touch these unless you know why.
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
		// --- Phase 1: do some "work" for round r ---
		var x uint64
		for i := uint64(0); i < 1000; i++ {
			x += i
		}
		_ = x

		// --- Barrier ---
		barrier.ArriveAndWait()

		// --- Invariant check: we should all agree we just finished round r ---
		seen := globalRound.Load()
		if seen != int32(r) {
			panic(fmt.Sprintf("thread %d saw round %d but expected %d", tid, seen, r))
		}
		if arrivedThisRound.Add(1) == int32(NThreads) {
			// Last one out advances the round and resets.
			arrivedThisRound.Store(0)
			globalRound.Store(int32(r + 1))
		}

		// --- Second barrier keeps everyone lockstep before next round ---
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
