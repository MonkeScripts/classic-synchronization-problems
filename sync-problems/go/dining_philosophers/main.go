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

func eat(s *Shared, pid int) {
	s.states[pid].Store(Hungry)

	// ======================================================================
	// TODO: Acquire resources here.
	// ======================================================================

	s.states[pid].Store(Eating)
	s.checkInvariant(pid)
	s.meals[pid].Add(1)
	jitter(3)
	s.states[pid].Store(Thinking)

	// ======================================================================
	// TODO: Release.
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
