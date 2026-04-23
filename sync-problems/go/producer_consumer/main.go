// Producer-Consumer — Go, channel variant
//
// Run with the race detector during development:
//     go run -race ./producer_consumer
//
// In Go, a buffered channel IS a bounded buffer. Full stop. There is no
// BoundedBuffer struct here, no mutex, no condvar. The whole "notFull /
// notEmpty" dance from the C++ version is replaced by ONE declaration:
//
//     items := make(chan string, BufferSize)
//
// - `items <- x` blocks when the channel is full (equivalent to
//   `wait(not_full)`).
// - `<-items` blocks when the channel is empty (equivalent to
//   `wait(not_empty)`).
// - `close(items)` wakes every ranger/receiver and drains cleanly.
//
// What's NOT free:
//   1. Who calls `close()`? Only the SENDING side is allowed to close.
//      With N producers, you need to coordinate so close happens exactly
//      once, and only after the LAST producer is done.
//      (Closing twice → panic. Sending on a closed channel → panic.)
//   2. `for item := range ch` is the idiomatic consumer — exits cleanly
//      when the channel is closed AND drained. No nullopt, no sentinel,
//      no explicit "done" flag. This alone is worth writing the version
//      for.
//
// The TODOs below walk you through those two points.
//
// Second-pass variants worth writing:
//   A. Balk-on-full: use `select { case items <- x: ...; default: ... }`
//      so producers drop items instead of blocking (turn this into a
//      best-effort log shipper).
//   B. Cancellation: add a `ctx context.Context` or `done chan struct{}`
//      and use `select` in both producer and consumer so shutdown can be
//      triggered externally, not just by "all producers finished."
//   C. Multi-stage pipeline: chain two channel-based stages to feel how
//      Go composes ("URLs → fetch → parsed pages → index").

package main

import (
	"fmt"
	"sync"
	"sync/atomic"
)

const (
	BufferSize       = 8
	NumProducers     = 3
	NumConsumers     = 2
	ItemsPerProducer = 50
)

func main() {
	// ======================================================================
	// The whole bounded buffer:
	// ======================================================================
	items := make(chan string, BufferSize)

	var produced, consumed atomic.Int32
	var pwg sync.WaitGroup // waits for producers
	var cwg sync.WaitGroup // waits for consumers

	// ----------------------------------------------------------------------
	// Producers
	// ----------------------------------------------------------------------
	for p := 0; p < NumProducers; p++ {
		pwg.Add(1)
		go func(pid int) {
			defer pwg.Done()
			for i := 0; i < ItemsPerProducer; i++ {
				// TODO:
				//   - build the item (e.g. "P%d#%d" of pid and i)
				//   - send it on `items`  (this blocks when buffer full
				//     — that IS the backpressure you wanted)
				//   - bump produced counter
				//   - small sleep to vary timing (optional but useful)

				item := fmt.Sprintf("P%d#%d", pid, i)
				items <- item
				produced.Add(1)

			}
		}(p)
	}

	// ----------------------------------------------------------------------
	// Close coordination
	//
	// The channel must be closed exactly once, AFTER the last producer is
	// done, and BEFORE the consumers give up ranging. Only a sender is
	// allowed to close. Closing twice panics.
	//
	// Idiomatic shape: one dedicated "closer" goroutine that waits for
	// producers on pwg and then closes `items`. Do NOT put the close()
	// inside any producer goroutine — none of them knows individually
	// whether it is the last one without extra coordination.
	//
	// TODO: start a goroutine that:
	//   - pwg.Wait()
	//   - close(items)
	// ----------------------------------------------------------------------

	go func() {
		pwg.Wait() // wait for producer to be done
		// close the channel
		close(items) // i forgot about this!
	}()

	// ----------------------------------------------------------------------
	// Consumers
	// ----------------------------------------------------------------------
	for c := 0; c < NumConsumers; c++ {
		cwg.Add(1)
		go func(cid int) {
			defer cwg.Done()
			// TODO:
			//   for item := range items {
			//       _ = item
			//       consumed.Add(1)
			//       // small sleep if you want to simulate work
			//   }
			//
			// The range-loop exits automatically when:
			//   - the channel is closed, AND
			//   - every buffered item has been drained.
			// You do NOT need a "is closed" check, a nullopt, or a
			// sentinel value. That's the whole point of this variant.
			for range items {
				consumed.Add(1)
			}

		}(c)
	}

	// Wait for consumers to finish draining the channel before we print.
	// (Producers are waited on by the closer goroutine; but we still need
	// to join them here too so their goroutines are fully torn down before
	// the process exits — `pwg.Wait()` is safe to call from multiple
	// places.)
	pwg.Wait()
	cwg.Wait()

	expected := int32(NumProducers * ItemsPerProducer)
	fmt.Printf("Produced=%d Consumed=%d Expected=%d\n",
		produced.Load(), consumed.Load(), expected)
	if produced.Load() != expected || consumed.Load() != expected {
		panic("mismatch — lost or double-counted items")
	}
}
