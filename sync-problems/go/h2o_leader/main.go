// H2O Water Factory — leader-election template (Go)
//
// Companion to ../h2o/ (daemon-goroutine version). Same problem, different
// strategy: instead of a long-running daemon, the OXYGEN ATOM is the leader
// for one molecule. Since each molecule has exactly 1 O, there is at most
// one leader at a time — no separate coordinator goroutine, no leak.
//
// Run with -race during development:
//   go run -race .
//
// Protocol (lecture demo 3):
//   hydrogen:
//     commit := make(chan struct{})
//     wf.precomH <- commit          // (precommit) submit yourself
//     <-commit                      // (commit) wait for leader's go
//     bondH(id)
//     commit <- struct{}{}          // (postcommit) tell leader I'm done
//
//   oxygen (the leader for this molecule):
//     <-wf.oxygenMutex              // become leader (cap-1 channel as a mutex)
//     h1 := <-wf.precomH            // (precommit) collect 2 H arrivals
//     h2 := <-wf.precomH
//     h1 <- struct{}{}              // (commit) signal both H to bond
//     h2 <- struct{}{}
//     bondO(id)
//     <-h1                          // (postcommit) wait for both H done
//     <-h2
//     wf.oxygenMutex <- struct{}{}  // step down — next O can lead
//
// Why this works without a daemon:
//   - Only one O can hold the oxygenMutex at a time → only one leader.
//   - precomH is a *shared* queue of waiting H atoms; the leader pulls
//     exactly 2 per molecule.
//   - precomO doesn't exist — the leader IS the oxygen for its molecule.
//
// Trade-off vs the daemon:
//   - No goroutine leak, no Destroy/context plumbing needed.
//   - oxygenMutex must be initialized with one token in the buffer so the
//     first oxygen can take it (see NewWaterFactory). Forgetting this
//     deadlocks immediately.
//
// Invariants enforced by the harness (same as daemon version):
//   - hInBond <= 2 always (panics on increment past 2 — catches ozone)
//   - oInBond <= 1 always (catches double-oxygen)
//   - maxTotalInBond should reach 3 (catches "no barrier" — atoms bonding solo)
//   - hTotalBonds == 2 * NMolecules, oTotalBonds == NMolecules

package main

import (
	"fmt"
	"math/rand"
	"sync"
	"sync/atomic"
	"time"
)

const (
	NMolecules = 50
	NHydrogen  = 2 * NMolecules
	NOxygen    = NMolecules
)

// ----- Invariant tracking (do not touch) -----
var (
	hInBond        atomic.Int32
	oInBond        atomic.Int32
	maxTotalInBond atomic.Int32
	hTotalBonds    atomic.Int32
	oTotalBonds    atomic.Int32
)

func updateMaxTotal() {
	total := hInBond.Load() + oInBond.Load()
	for {
		cur := maxTotalInBond.Load()
		if total <= cur || maxTotalInBond.CompareAndSwap(cur, total) {
			return
		}
	}
}

func bondH(id int) {
	h := hInBond.Add(1)
	if h > 2 {
		panic(fmt.Sprintf("ozone! %d H atoms bonding simultaneously", h))
	}
	updateMaxTotal()
	time.Sleep(2 * time.Millisecond)
	updateMaxTotal()
	time.Sleep(1 * time.Millisecond)
	hInBond.Add(-1)
	hTotalBonds.Add(1)
	_ = id
}

func bondO(id int) {
	o := oInBond.Add(1)
	if o > 1 {
		panic(fmt.Sprintf("more than 1 O bonding (%d O atoms)", o))
	}
	updateMaxTotal()
	time.Sleep(2 * time.Millisecond)
	updateMaxTotal()
	time.Sleep(1 * time.Millisecond)
	oInBond.Add(-1)
	oTotalBonds.Add(1)
	_ = id
}

// ==========================================================================
// TODO: Implement WaterFactory using the LEADER strategy (oxygen as leader).
//
// Suggested fields:
//   oxygenMutex chan struct{}     // capacity 1, used as a non-reentrant mutex
//   precomH     chan chan struct{}
//
// Don't forget: pre-fill oxygenMutex with one token in NewWaterFactory so the
// first oxygen can acquire it. (If you forget, the very first oxygen blocks
// forever waiting for a token nobody released.)
// ==========================================================================


// Do not need central manager, instead oxygen is the central manager. 
// We will use a semaphore to make sure oxygen is handled one by one
// the oxygen method handles the hydrogen requests
type WaterFactory struct {
	// TODO: fields
	RequestH chan chan struct{}
	OxySem chan struct{}
}

func NewWaterFactory() *WaterFactory {
	// TODO: build channels and pre-fill oxygenMutex with one token.
	return &WaterFactory{
		RequestH : make(chan chan struct{}),
		OxySem : make(chan struct{}, 1),

	}
}

func (wf *WaterFactory) hydrogen(id int) {
	// TODO: precommit / commit / bondH(id) / postcommit
	commit := make(chan struct{})
	wf.RequestH <- commit
	<- commit
	bondH(id)
	commit <- struct {}{} // need to see how its done
}

func (wf *WaterFactory) oxygen(id int) {
	// TODO: become leader, collect 2 H, signal all 3, bondO(id), wait done, step down
	wf.OxySem <- struct{}{}
	h1 := <- wf.RequestH
	h2 := <- wf.RequestH
	h1 <- struct{}{}
	h2 <- struct{}{}
	bondO(id)
	<- h1
	<- h2
	<- wf.OxySem

}

func main() {
	wf := NewWaterFactory()
	var wg sync.WaitGroup
	rng := rand.New(rand.NewSource(42))
	start := time.Now()

	for i := 0; i < NHydrogen; i++ {
		time.Sleep(time.Duration(rng.Intn(5)) * time.Millisecond)
		wg.Add(1)
		go func(id int) { defer wg.Done(); wf.hydrogen(id) }(i)
	}
	for i := 0; i < NOxygen; i++ {
		time.Sleep(time.Duration(rng.Intn(5)) * time.Millisecond)
		wg.Add(1)
		go func(id int) { defer wg.Done(); wf.oxygen(id) }(i)
	}
	wg.Wait()
	elapsed := time.Since(start)

	hb := hTotalBonds.Load()
	ob := oTotalBonds.Load()
	mt := maxTotalInBond.Load()
	fmt.Printf("H bonds: %d/%d  O bonds: %d/%d  max simultaneous: %d  time: %v\n",
		hb, NHydrogen, ob, NOxygen, mt, elapsed)

	ok := true
	if hb != NHydrogen {
		fmt.Println("MISSING H BONDS")
		ok = false
	}
	if ob != NOxygen {
		fmt.Println("MISSING O BONDS")
		ok = false
	}
	if mt != 3 {
		fmt.Printf("max simultaneous should be 3, got %d — atoms bonding without all 3 present\n", mt)
		ok = false
	}
	if !ok {
		panic("invariants failed")
	}
}
