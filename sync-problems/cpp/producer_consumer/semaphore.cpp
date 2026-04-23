// Producer-Consumer — variant: counting semaphores (C++20)
//
// Same scenario as double_cond.cpp. Same BoundedBuffer<T> interface. The
// difference: instead of one mutex + two condition_variables, you use
// two std::counting_semaphore<>s plus a mutex that ONLY protects the
// queue's internal structure, not the wait logic.
//
// Mental model (lecture pseudocode directly):
//     spaces_sem starts at BUFFER_SIZE  (tickets for "there's an empty slot")
//     items_sem  starts at 0            (tickets for "there's an item")
//     mut protects queue_ operations only
//
// Producer:
//     wait(spaces_sem)     // take a ticket for one empty slot
//     mut.lock(); push item into queue; mut.unlock()
//     signal(items_sem)    // emit a ticket saying "an item is available"
//
// Consumer mirrors it:
//     wait(items_sem)
//     mut.lock(); pop item; mut.unlock()
//     signal(spaces_sem)
//
// Why this is worth writing: you'll feel the elegance — no predicates,
// no spurious-wakeup worry — AND you'll hit the nasty part: clean shutdown.
// Semaphores don't have broadcast. To wake N stuck waiters on a closed
// buffer, you have to release enough permits for each of them to
// re-check a `closed_` flag and bail. That re-check is the whole
// shutdown dance in this version.
//
// Pitfalls specific to this style:
//   1. Double-count on close: if you release permits in close(), a
//      late-arriving producer can consume one of those and try to push,
//      bypassing the closed check. Re-check `closed_` AFTER every
//      semaphore.acquire() and before modifying queue_.
//   2. max-count on std::counting_semaphore<> defaults to
//      std::numeric_limits<ptrdiff_t>::max() — fine here. If you want
//      to bound the max explicitly, use counting_semaphore<BUFFER_SIZE>.
//   3. Release order matters less than with condvars, but you MUST
//      release spaces_sem AFTER the item is actually popped — otherwise
//      a producer could see a ticket that still doesn't have a free slot.
//
// Compare against double_cond.cpp:
//   - Lines of code for push/pop (semaphore should be fewer).
//   - Complexity of close(): which version is harder to get right?
//   - Performance: which is faster at high contention? (Benchmark both.)

#include <atomic>
#include <chrono>
#include <deque>
#include <iostream>
#include <mutex>
#include <optional>
#include <random>
#include <semaphore>
#include <string>
#include <thread>
#include <vector>

using namespace std::chrono_literals;

constexpr std::size_t BUFFER_SIZE = 8;
constexpr int NUM_PRODUCERS = 3;
constexpr int NUM_CONSUMERS = 2;
constexpr int ITEMS_PER_PRODUCER = 50;
constexpr int MAX_PRODUCERS_WAITERS = NUM_PRODUCERS;                                                                                                                                                                                               
constexpr int MAX_CONSUMERS_WAITERS = NUM_CONSUMERS;  

template <typename T>
class BoundedBuffer {
public:
    explicit BoundedBuffer(std::size_t capacity)
        : capacity_(capacity),
          // TODO: are these initial counts right for your strategy?
          spaces_sem_(static_cast<std::ptrdiff_t>(capacity)),
          items_sem_(0) {}

    // Blocks until space is available OR the buffer is closed.
    // Returns false if closed (item was NOT inserted).
    bool push(T item) {
        // ================================================================
        // TODO (semaphore variant):
        //   - spaces_sem_.acquire()         // take one "empty slot" ticket
        //   - if closed_ is true at this point, release the ticket (so
        //     shutdown doesn't leak permits) and return false.
        //   - lock mut_, queue_.push_back(std::move(item)), unlock mut_
        //   - items_sem_.release()           // advertise the new item
        //   - return true
        //
        // Q: why re-check closed_ AFTER acquire() instead of before?
        // Q: does it matter if you release items_sem_ inside or outside
        //    the mutex? Reason carefully — think about what a waiting
        //    consumer sees.
        // ================================================================
        spaces_sem_.acquire();
        if (closed_) {
            spaces_sem_.release();
            return false;
        }
        {
            std::unique_lock<std::mutex> lk {mut_};
            queue_.push_back(std::move(item));
        }
        items_sem_.release();
        return true;
    }

    // Blocks until an item is available. Returns nullopt if buffer is
    // closed AND empty — this is how consumers know to exit.
    std::optional<T> pop() {
        // ================================================================
        // TODO (mirror image of push):
        //   - items_sem_.acquire()
        //   - if closed_ && queue_ empty: release items_sem_ back and
        //     return std::nullopt. (Subtle: if you don't release, the
        //     permits don't balance across shutdown.)
        //   - lock mut_, take queue_.front() (move!), pop_front(), unlock
        //   - spaces_sem_.release()
        //   - return item
        // ================================================================
        items_sem_.acquire();
        std::unique_lock<std::mutex> lk {mut_};
        if (closed_ and queue_.empty()) {
            items_sem_.release();
            return std::nullopt;
        }
        
        auto item = std::move(queue_.front());
        queue_.pop_front();
        spaces_sem_.release();
        return item;
    }

    // Wake every stuck waiter so they can observe closed_ and bail.
    // This is the tricky part of the semaphore version — no broadcast
    // primitive, so we have to release "enough" permits.
    void close() {
        {
            std::scoped_lock lock(mut_);
            closed_ = true;
        }
        // ================================================================
        // TODO: release enough permits on BOTH semaphores that every
        // possibly-blocked producer and consumer wakes up. How many is
        // "enough"? In the worst case: every producer could be blocked on
        // spaces_sem_ AND every consumer on items_sem_. Release at least
        // that many on each side. Your push/pop logic MUST re-release the
        // permit it consumed if it observes closed_ — otherwise repeated
        // close() calls (or late arrivals) starve the next waiter.
        //
        // Hint: NUM_PRODUCERS + some-slack for spaces_sem_,
        //       NUM_CONSUMERS + some-slack for items_sem_.
        // ================================================================S
        int N_enough = std::max(MAX_CONSUMERS_WAITERS, MAX_PRODUCERS_WAITERS);
        for (int i = 0; i < N_enough; ++i) {
            spaces_sem_.release();
            items_sem_.release();
        }

    }

    std::size_t capacity() const { return capacity_; }

private:
    std::size_t capacity_;
    std::deque<T> queue_;
    std::mutex mut_;
    std::counting_semaphore<> spaces_sem_;
    std::counting_semaphore<> items_sem_;
    std::atomic<bool> closed_ = false;
};

int main() {
    BoundedBuffer<std::string> buffer(BUFFER_SIZE);

    std::atomic<int> produced{0};
    std::atomic<int> consumed{0};
    std::atomic<int> producers_done{0};

    std::vector<std::thread> producers;
    for (int p = 0; p < NUM_PRODUCERS; ++p) {
        producers.emplace_back([&, pid = p] {
            std::mt19937 rng(pid);
            for (int i = 0; i < ITEMS_PER_PRODUCER; ++i) {
                std::string item = "P" + std::to_string(pid) + "#" + std::to_string(i);
                if (!buffer.push(std::move(item))) break;
                produced.fetch_add(1);
                std::this_thread::sleep_for(
                    std::chrono::milliseconds(rng() % 3));
            }
            if (producers_done.fetch_add(1) + 1 == NUM_PRODUCERS) {
                buffer.close();
            }
        });
    }

    std::vector<std::thread> consumers;
    for (int c = 0; c < NUM_CONSUMERS; ++c) {
        consumers.emplace_back([&, cid = c] {
            std::mt19937 rng(cid + 100);
            while (auto item = buffer.pop()) {
                (void)*item; // pretend to flush
                consumed.fetch_add(1);
                std::this_thread::sleep_for(
                    std::chrono::milliseconds(rng() % 5));
            }
        });
    }

    for (auto& t : producers) t.join();
    for (auto& t : consumers) t.join();

    int expected = NUM_PRODUCERS * ITEMS_PER_PRODUCER;
    std::cout << "Produced=" << produced.load()
              << " Consumed=" << consumed.load()
              << " Expected=" << expected << "\n";
    if (produced.load() != expected || consumed.load() != expected) {
        std::cerr << "MISMATCH — lost or double-counted items!\n";
        return 1;
    }
}
