// Barbershop — starter template (Go)
//
// Scenario: food-truck line with Chairs waiting slots.
//   - TotalCustomers arrive over time (Poisson-ish).
//   - If all chairs are occupied when a customer shows up, they BALK
//     (leave immediately) and count toward `balked`.
//   - Barber sleeps when no customers are waiting, wakes on arrival.
//
// Go gives you two natural shapes. Pick ONE, get it green, then do the other.
//   1. Channel-based (feels idiomatic):
//         chairs:      buffered chan of size Chairs  (who's sitting)
//         barberReady: unbuffered chan struct{}      (rendezvous)
//         customerDone, barberDone: unbuffered       (handshake)
//      Note: Go `select` picks randomly among ready cases, so this version
//      does NOT give you FIFO. For FIFO, see version 2.
//
//   2. sync.Mutex + *sync.Cond + an explicit queue (slice or list.List).
//      More code, but FIFO is trivial. Mirrors the C++/Rust versions.
//
// Invariants (enforced in main()):
//   - served + balked == TotalCustomers (no customer vanishes)
//   - customer count never exceeds Chairs (handled by buffered chan capacity
//     in v1, or the if-full check in v2)
//
// Stretch: multiple barbers; priority customers.

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
// TODO: Flesh out the channel-based shape, OR replace this struct entirely
// with a mutex+cond version. The fields below match version 1.
// ==========================================================================
type Barbershop struct {
	chairs       chan int      // buffered (capacity Chairs); a customer ID parks here
	barberReady  chan struct{} // unbuffered: barber signals "I'm ready for you"
	customerDone chan struct{} // unbuffered: customer signals "cut finished"
	barberDone   chan struct{} // unbuffered: barber signals "you can leave"
	shutdown     chan struct{}
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

// Customer returns true if served, false if balked.
func (bs *Barbershop) Customer(id int) bool {
		select {
		case <- bs.shutdown:
			return false
		case bs.chairs <- id:
			<-bs.barberReady
			time.Sleep(2 * time.Millisecond) // cut hair
			<- bs.barberDone
			bs.customerDone <- struct{}{}
			return true
		default:
			return false //balk
		}
}

func (bs *Barbershop) Barber() {
	for {
		select {
		case <-bs.shutdown:
			return
		// case id := <-bs.chairs: UNUSED variables cannot compile
		case <-bs.chairs:
			bs.barberReady <- struct{}{}
			time.Sleep(2 * time.Millisecond) // cut hair
			bs.barberDone <- struct{}{}
			<-bs.customerDone

		}
	}
}

func (bs *Barbershop) Shutdown() { close(bs.shutdown) }

func main() {
	shop := NewBarbershop()
	go shop.Barber()

	var served, balked atomic.Int32
	var wg sync.WaitGroup

	rng := rand.New(rand.NewSource(42))
	for i := 0; i < TotalCustomers; i++ {
		// Poisson-ish arrivals: mean 20ms between customers.
		time.Sleep(time.Duration(rng.Intn(40)) * time.Millisecond)
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
	if total != int32(TotalCustomers) {
		panic("lost customers — bug in rendezvous")
	}
}
