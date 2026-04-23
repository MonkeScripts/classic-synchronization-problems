// Producer-Consumer — starter template (Go)
//
// In Go, "bounded buffer" is literally `make(chan T, N)`. If you jump
// straight to that, you skip the learning. So:
//
//   Version 1 (this file): mutex + *sync.Cond + container/list. This is
//       the shape C++/Rust/Java developers write and is what the lecture
//       pseudocode (wait(notFull)/signal(notEmpty)) translates to.
//
//   Version 2 (second pass, REWRITE this file): `items := make(chan string,
//       BufferSize)`. Producers do `items <- item`; consumers do
//       `for item := range items { ... }`. Only the LAST producer closes.
//       Compare line counts and reflect on what the channel hides.
//
//   Version 3 (stretch): lock-free MPMC queue. Big topic.
//
// Classic bugs to watch for while filling this in:
//   - `if` instead of `for` around cv.Wait — spurious wakeups are a thing.
//   - Signalling/broadcasting to the wrong condvar (send on notEmpty when
//     you emptied a slot, not filled one).
//   - Shutdown hangs: a consumer sleeping on notEmpty after the last
//     producer closed — close() below broadcasts both cvs for this reason.

package main

import (
	"container/list"
	"fmt"
	"sync"
	"sync/atomic"
	"time"
)

const (
	BufferSize       = 8
	NumProducers     = 3
	NumConsumers     = 2
	ItemsPerProducer = 50
)

type BoundedBuffer struct {
	mu       sync.Mutex
	notFull  *sync.Cond
	notEmpty *sync.Cond
	q        *list.List
	capacity int
	closed   bool
}

func NewBoundedBuffer(capacity int) *BoundedBuffer {
	b := &BoundedBuffer{q: list.New(), capacity: capacity}
	b.notFull = sync.NewCond(&b.mu)
	b.notEmpty = sync.NewCond(&b.mu)
	return b
}

// Push blocks while the buffer is full. Returns false if the buffer was
// closed before space became available (item was NOT inserted).
func (b *BoundedBuffer) Push(item string) bool {
	// ================================================================
	// TODO:
	//   - b.mu.Lock(); defer b.mu.Unlock()
	//   - for b.q.Len() == b.capacity && !b.closed { b.notFull.Wait() }
	//   - if b.closed: return false
	//   - b.q.PushBack(item)
	//   - b.notEmpty.Signal()
	//   - return true
	// ================================================================
	_ = item
	return false
}

// Pop blocks while the buffer is empty. Returns ("", false) when the buffer
// is closed AND drained — that's how consumers know to exit their loop.
func (b *BoundedBuffer) Pop() (string, bool) {
	// ================================================================
	// TODO (mirror-image of Push):
	//   - b.mu.Lock(); defer b.mu.Unlock()
	//   - for b.q.Len() == 0 && !b.closed { b.notEmpty.Wait() }
	//   - if b.q.Len() == 0 && b.closed: return "", false
	//   - item := b.q.Front(); b.q.Remove(item)
	//   - b.notFull.Signal()
	//   - return item.Value.(string), true
	// ================================================================
	return "", false
}

// Close is the boilerplate part — you don't need to change this.
// It wakes every waiter on both condition variables so nobody is stuck
// sleeping after the producers are done.
func (b *BoundedBuffer) Close() {
	b.mu.Lock()
	defer b.mu.Unlock()
	b.closed = true
	b.notFull.Broadcast()
	b.notEmpty.Broadcast()
}

func main() {
	buf := NewBoundedBuffer(BufferSize)
	var produced, consumed, producersDone atomic.Int32
	var pwg, cwg sync.WaitGroup

	for p := 0; p < NumProducers; p++ {
		pwg.Add(1)
		go func(pid int) {
			defer pwg.Done()
			for i := 0; i < ItemsPerProducer; i++ {
				if !buf.Push(fmt.Sprintf("P%d#%d", pid, i)) {
					return
				}
				produced.Add(1)
				time.Sleep(time.Duration(pid%3) * time.Millisecond)
			}
			// Only the last producer out closes the buffer.
			if producersDone.Add(1) == int32(NumProducers) {
				buf.Close()
			}
		}(p)
	}

	for c := 0; c < NumConsumers; c++ {
		cwg.Add(1)
		go func(cid int) {
			defer cwg.Done()
			for {
				item, ok := buf.Pop()
				if !ok {
					return
				}
				_ = item // pretend to flush
				consumed.Add(1)
			}
		}(c)
	}

	pwg.Wait()
	cwg.Wait()

	expected := int32(NumProducers * ItemsPerProducer)
	fmt.Printf("Produced=%d Consumed=%d Expected=%d\n",
		produced.Load(), consumed.Load(), expected)
	if produced.Load() != expected || consumed.Load() != expected {
		panic("mismatch — lost or double-counted items")
	}
}
