// FIFO Semaphore — starter template (Go)
//
// Scenario (lecture T7.2): build a counting semaphore that wakes waiters in
// arrival order.
//
// Run with -race during development:
//   go run -race .
//
// Strategies (lecture demos 6-7):
//   1. Buffered channel as semaphore: cap == max permits, sends increment,
//      receives decrement. Idiomatic Go but FIFO is NOT guaranteed by the
//      Go spec — the runtime *happens to* serve channel waiters FIFO in
//      practice but it's not normative.
//   2. Daemon goroutine + queue of per-waiter chans: each acquirer creates
//      a chan struct{}, sends it on a queue chan, then receives on it to
//      unblock. Daemon serves arrivals strictly FIFO by construction.
//   3. Atomic ticket queue + sync.Cond (mirror of the C++ Task 1 solution).
//
// Invariant: threads spawned in spawn order with ARRIVAL_SPACING_MS gaps
// should wake in spawn order. wake_indices[i] should equal i.

package main

import (
	"fmt"
	"sync"
	"sync/atomic"
	"time"
	"container/list"
)

const (
	NThreads         = 16
	InitialCount     = 0
	ArrivalSpacingMs = 10
)
type chanQueue struct{ list.List }

func NewChanQueue() *chanQueue {
	q := new(chanQueue)
	q.Init()
	return q
}

func (q *chanQueue) Pop() chan struct{} {
	ele := q.Front()
	q.Remove(ele)
	return ele.Value.(chan struct{})
}

// ==========================================================================
// TODO: Implement FifoSemaphore.
//
// API contract:
//   New(initial)
//   Acquire(): block until count > 0, then decrement. FIFO order.
//   Release(): increment; oldest waiter wakes.
// ==========================================================================
type FifoSemaphore struct {
	// TODO: fields
	acquireCh chan chan struct{}
	releaseCh chan struct{}
}

func NewFifoSemaphore(initial int) *FifoSemaphore {
	fifoSem := new(FifoSemaphore)
	fifoSem.releaseCh = make(chan struct{}, 100) // why is this buffered
	fifoSem.acquireCh = make (chan chan struct {}, 100)
	go func() {
		count := initial
		waiters := NewChanQueue()
		for {
			select {
				case <-fifoSem.releaseCh:
				if waiters.Len() > 0 {
					ch := waiters.Pop()
					ch <- struct{}{}
				} else {
					count++
				}
				case ch:= <-fifoSem.acquireCh:
				if count > 0 {
					ch <- struct{}{}
					count -= 1
				} else {
					waiters.PushBack(ch)
				}
			}
		}
	}()
	return fifoSem
}

func (s *FifoSemaphore) Acquire() {
	ch := make(chan struct{})
	s.acquireCh <- ch
	<-ch
}

func (s *FifoSemaphore) Release() {
	s.releaseCh <- struct{}{}
}

// ----- Invariant tracking (do not touch) -----
var globalWake atomic.Int32

func main() {
	sem := NewFifoSemaphore(InitialCount)
	wakeIndices := make([]atomic.Int32, NThreads)
	for i := range wakeIndices {
		wakeIndices[i].Store(-1)
	}

	var wg sync.WaitGroup
	start := time.Now()

	for i := 0; i < NThreads; i++ {
		wg.Add(1)
		go func(i int) {
			defer wg.Done()
			time.Sleep(time.Duration(i*ArrivalSpacingMs) * time.Millisecond)
			sem.Acquire()
			wake := globalWake.Add(1) - 1
			wakeIndices[i].Store(wake)
			time.Sleep(2 * time.Millisecond)
			sem.Release()
		}(i)
	}

	// Wait for everyone to be queued, then kick off the chain.
	time.Sleep(time.Duration((NThreads+2)*ArrivalSpacingMs) * time.Millisecond)
	sem.Release()

	wg.Wait()
	elapsed := time.Since(start)

	violations := 0
	for i := 0; i < NThreads; i++ {
		w := wakeIndices[i].Load()
		if int(w) != i {
			fmt.Printf("FIFO violation: thread %d woke at index %d\n", i, w)
			violations++
		}
	}
	fmt.Printf("Took %v, %d FIFO violations\n", elapsed, violations)
	if violations > 0 {
		panic("FIFO violations")
	}
}
