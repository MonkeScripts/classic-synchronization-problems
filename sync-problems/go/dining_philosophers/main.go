// Dining Philosophers — starter template (Go)
//
// Run with the race detector ALWAYS during development:
//     go run -race .
//
// Go gives you two natural weapons: sync.Mutex (lock-based, like C++)
// and channels (message-passing). Try BOTH styles — the lecture shows
// channel approaches (odd/even ring, footman) but sync.Mutex is also fine.
//
// Strategies worth implementing:
//   1. Naive []sync.Mutex per chopstick — deadlocks. Prove it.
//   2. Asymmetric ordering.
//   3. Channel-per-chopstick (chopstick "token" passed via channel).
//   4. Footman channel admitting only N-1 eaters.
//   5. Odd/even ring (as in lecture slide 31).

package main

import (
	"fmt"
	"math/rand"
	"sync"
	"sync/atomic"
	"time"
)

const (
	N                    = 5
	MealsPerPhilosopher  = 50
)

const (
	Thinking = iota
	Hungry
	Eating
)

type Shared struct {
	states [N]atomic.Int32
	meals  [N]atomic.Int32

	// TODO: add your chopsticks / channels / footman here.
	// Example (mutex-based):
	// chopsticks [N]sync.Mutex
	// Example (channel-based):
	// chopsticks [N]chan struct{}
	chopstickChs [N]chan struct{}
}

func (s *Shared) Init() {
	for i := range s.chopstickChs {
		s.chopstickChs[i] = make(chan struct{}, 1)
		s.chopstickChs[i] <- struct{}{}
	}
}

func (s *Shared) checkInvariant(pid int) {
	left := (pid + N - 1) % N
	right := (pid + 1) % N
	if s.states[left].Load() == Eating {
		panic(fmt.Sprintf("philosopher %d eating while left %d is eating", pid, left))
	}
	if s.states[right].Load() == Eating {
		panic(fmt.Sprintf("philosopher %d eating while right %d is eating", pid, right))
	}
}

func jitter(maxMs int) {
	time.Sleep(time.Duration(rand.Intn(maxMs+1)) * time.Millisecond)
}

func think(s *Shared, pid int) {
	s.states[pid].Store(Thinking)
	jitter(3)
}
func (s *Shared) getLeftChopstickCh(pid int) chan struct{} {
	return s.chopstickChs[pid]
	
}
func (s *Shared) getRightChopstickCh(pid int) chan struct{} {
		return s.chopstickChs[(pid + 1) % N ]
}

func (s *Shared) getEvenIdxChopstickCh(pid int) chan struct{} {
	if pid % 2 == 0 {
		return s.getLeftChopstickCh(pid)
	} else {
		return s.getRightChopstickCh(pid)
	}
	
}
func (s *Shared) getOddIdxChopstickCh(pid int) chan struct{} {
	if pid % 2 == 1 {
		return s.getLeftChopstickCh(pid)
	} else {
		return s.getRightChopstickCh(pid)
	}

}

func eat(s *Shared, pid int) {
	s.states[pid].Store(Hungry)

	// ======================================================================
	// TODO: Acquire resources here.
	// ======================================================================
	evenIdxCh := s.getEvenIdxChopstickCh(pid)
	oddIdxCh := s.getOddIdxChopstickCh(pid)
	<- evenIdxCh
	<- oddIdxCh

	s.states[pid].Store(Eating)
	s.checkInvariant(pid)
	s.meals[pid].Add(1)
	jitter(3)
	s.states[pid].Store(Thinking)

	evenIdxCh <- struct{}{}
	oddIdxCh <- struct{}{}

	// ======================================================================
	// TODO: Release.
	// ======================================================================
}

// =========================================================================
// Strategy: try-and-back-off (channel equivalent of C++ std::scoped_lock)
// =========================================================================
// Block on one chopstick, then NON-BLOCKING try the other. If the second
// fails, release the first and retry in the OTHER ORDER. Either both
// chopsticks are held together at the moment of `break acquire`, or
// neither is held at the bottom of the loop body — never one held while
// some other philosopher waits on it as their first.
//
// Compare to the odd/even ring (eat() above):
//   - Odd/even ring: prevents the cycle STRUCTURALLY in the lock-acquire
//     graph — odd philosophers go in the opposite direction.
//   - Try-and-back-off (this): the cycle can briefly form, but the
//     non-blocking try unwinds it before it actualises. Equivalent to
//     C++'s std::lock(a,b) / std::scoped_lock{a,b} algorithm.
//
// Liveness note: this can LIVELOCK in pathological scheduling — every
// philosopher grabs their first, every try fails, every philosopher
// releases and loops. With Go's channel FIFO and scheduler jitter this
// is rare in practice; production code should add a randomized backoff
// sleep between iterations to prove livelock impossible.
//
// Go syntax used:
//   - LABELED BREAK: `acquire:` is a label on the for-loop; `break acquire`
//     escapes the for-loop from inside the select. A bare `break` would
//     only escape the select case, leaving the for-loop running.
//   - NON-BLOCKING SELECT: `select { case <-ch: ...; default: ... }` is
//     the idiom for "try to receive; if you can't right now, do default."
//     Without `default:`, select blocks until one of the cases is ready.

func eatTryBackoff(s *Shared, pid int) {
	s.states[pid].Store(Hungry)

	leftCh := s.getLeftChopstickCh(pid)
	rightCh := s.getRightChopstickCh(pid)

	// ======================================================================
	// TODO: Acquire both chopsticks via try-and-back-off.
	// ======================================================================
acquire:
	for {
		<- leftCh
		select {
		case <-rightCh:
			break acquire
		default:

		}
		// reload leftCh
		leftCh <- struct {}{}
		<- rightCh 
		select {
		case <-leftCh:
			break acquire
		default:
		}
		// reload rigthCh
		rightCh <- struct{}{}


		// --- Phase A: try left-first ---
		// TODO: blocking receive on leftCh        (`<-leftCh`)
		// TODO: non-blocking select on rightCh:
		//         case <-rightCh:    break acquire     // both held — done
		//         default:                              // give up, fall through
		// TODO: put leftCh back                   (`leftCh <- struct{}{}`)

		// --- Phase B: try right-first (symmetric) ---
		// TODO: blocking receive on rightCh
		// TODO: non-blocking select on leftCh:
		//         case <-leftCh:     break acquire
		//         default:
		// TODO: put rightCh back

		// (Optional: jitter(1) here to break livelock under stress.)
	}

	s.states[pid].Store(Eating)
	s.checkInvariant(pid)
	s.meals[pid].Add(1)
	jitter(3)
	s.states[pid].Store(Thinking)

	leftCh <- struct{}{}
	rightCh <- struct{}{}

	// ======================================================================
	// TODO: Release both chopsticks (send tokens back into both channels).
	// ======================================================================
}

func philosopher(s *Shared, pid int, wg *sync.WaitGroup) {
	defer wg.Done()
	for i := 0; i < MealsPerPhilosopher; i++ {
		think(s, pid)
		eat(s, pid)
	}
}

func main() {
	s := &Shared{}

	// TODO: initialize your chopsticks/channels if needed.
	// Example for channel-based:
	// for i := range s.chopsticks {
	//     s.chopsticks[i] = make(chan struct{}, 1)
	//     s.chopsticks[i] <- struct{}{}  // prime each channel with one token
	// }
	s.Init()

	var wg sync.WaitGroup
	start := time.Now()
	for pid := 0; pid < N; pid++ {
		wg.Add(1)
		go philosopher(s, pid, &wg)
	}
	wg.Wait()
	elapsed := time.Since(start)

	fmt.Printf("Done in %v\n", elapsed)
	meals := [N]int32{}
	min, max := int32(1<<30), int32(0)
	for i := 0; i < N; i++ {
		meals[i] = s.meals[i].Load()
		if meals[i] < min {
			min = meals[i]
		}
		if meals[i] > max {
			max = meals[i]
		}
	}
	fmt.Printf("Meals per philosopher: %v\n", meals)
	fmt.Printf("min=%d max=%d spread=%d\n", min, max, max-min)
}
