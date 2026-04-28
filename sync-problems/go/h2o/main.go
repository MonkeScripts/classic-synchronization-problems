// H2O Water Factory — starter template (Go)
//
// Scenario (lecture T7.1): when 2 H + 1 O atoms arrive, they bond. Bond
// must satisfy: only 3 atoms in the critical section at once, AND no
// bonding starts until all 3 are present.
//
// Run with the race detector during development:
//   go run -race .
//
// Strategies (lecture demos 2-3):
//   1. Daemon goroutine that orchestrates the two-phase commit:
//        precommit:  receive arrival from 2 H + 1 O via chan chan struct{}
//        commit:     send () to all 3
//        postcommit: receive () from all 3
//      Trade-off: leaks a goroutine if the factory is dropped. Add a Destroy
//      method with a context to clean up.
//   2. Leader election (oxygen as leader): oxygen takes a leadership
//      mutex/channel, collects 2 H precommits via the shared chan, signals
//      all 3, bonds, waits for both H done, releases. No daemon, no leak.
//
// Invariants enforced by the harness:
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
// TODO: Implement WaterFactory.
//
// API contract:
//   hydrogen(id): coordinate, then call bondH(id)
//   oxygen(id):   coordinate, then call bondO(id)
//
// Suggested fields (daemon strategy):
//   precomH chan chan struct{}
//   precomO chan chan struct{}
//
// Suggested fields (leader strategy):
//   oxygenMutex chan struct{}     // 1-buffered, used as a non-reentrant lock
//   precomH     chan chan struct{}
// ==========================================================================

// Key IDEA: We have a central coordinator that checks whether we have sufficient elements for H2O
// Individual H and O sends a request to bond. This Request is in the form of chan struct{}
// The central coordinator would put the reply into the channel in request. 
// Once indiv atoms receive the response, they would then call bond()
type WaterFactory struct {
	RequestH chan chan struct{}
	RequestO chan chan struct{}
}

func NewWaterFactory() *WaterFactory {
	wf := &WaterFactory{
		RequestH : make(chan chan struct{}),
		RequestO : make(chan chan struct{}),
	}
	go centralManager(wf)
	return wf
}

func centralManager(wf *WaterFactory) {
	// can just directly take the request this way
	for {
		h1 := <-wf.RequestH
		h2 := <-wf.RequestH
		o := <-wf.RequestO
		// Give a response, "inject messages into the response"
		// consumed by indiv atom methods
		h1 <- struct{}{}
		h2 <- struct{}{}
		o <- struct{}{}
		// flush from next wave. Waits on injection of struct{}{} from indiv atoms
		<- h1
		<- h2
		<- o
	}

}

func (wf *WaterFactory) hydrogen(id int) {
	// TODO: coordinate, then bondH(id)
	commit := make(chan struct{})
	//wf.RequestH refers to the COMMON CHANNEL OF CHANNELS so we should make a temp channel instead to handle per atom methods
	wf.RequestH <- commit
	<-commit
	bondH(id)
	commit <- struct {}{}

}

func (wf *WaterFactory) oxygen(id int) {
	// TODO: coordinate, then bondO(id)
	commit := make(chan struct{})
	wf.RequestO <- commit
	<- commit
	bondO(id)
	commit <- struct {}{}

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
