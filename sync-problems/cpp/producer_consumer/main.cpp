// Producer-Consumer — starter template (C++20)
//
// Scenario: log aggregator.
//   - NUM_PRODUCERS worker threads emit log lines.
//   - Bounded ring buffer of size BUFFER_SIZE.
//   - NUM_CONSUMERS flusher threads batch-write to stdout.
//   - Ends cleanly: each producer emits ITEMS_PER_PRODUCER items,
//     then signals done. Consumers exit when producers done AND buffer empty.
//
// Your job: fill in BoundedBuffer<T>::push and BoundedBuffer<T>::pop.
//
// Classic pitfalls to watch for:
//   - Using `if` instead of `while` around cv.wait (spurious wakeups).
//   - Signaling outside the lock causing lost wakeups (rare but possible).
//   - Forgetting that consumers need a way to shut down (poison pill / flag).
//
// Alternative implementations to try on second pass:
//   1. Pure condition_variable (shown here).
//   2. Counting semaphores only (space/items semaphores + mutex for buffer).
//   3. Lock-free SPSC / MPMC queue (big topic — stretch).

#include <atomic>
#include <chrono>
#include <condition_variable>
#include <deque>
#include <iostream>
#include <mutex>
#include <optional>
#include <random>
#include <string>
#include <thread>
#include <vector>

using namespace std::chrono_literals;

constexpr std::size_t BUFFER_SIZE = 8;
constexpr int NUM_PRODUCERS = 3;
constexpr int NUM_CONSUMERS = 2;
constexpr int ITEMS_PER_PRODUCER = 50;

template <typename T>
class BoundedBuffer {
public:
    explicit BoundedBuffer(std::size_t capacity) : capacity_(capacity) {}

    // Blocks until space is available OR the buffer is closed.
    // Returns false if closed (item was NOT inserted).
    bool push(T item) {
        (void)item;
        // ================================================================
        // TODO:
        //   - unique_lock on mut_
        //   - wait on not_full_ while (queue is full AND !closed_)
        //   - if closed_: return false
        //   - push_back item
        //   - unlock (or let destructor) and notify_one on not_empty_
        //   - return true
        // ================================================================
        return false; // placeholder
    }

    // Blocks until an item is available. Returns nullopt if buffer is
    // closed AND empty — this is how consumers know to exit.
    std::optional<T> pop() {
        // ================================================================
        // TODO:
        //   - unique_lock on mut_
        //   - wait on not_empty_ while (queue is empty AND !closed_)
        //   - if queue empty AND closed_: return nullopt
        //   - pop front item
        //   - notify_one on not_full_
        //   - return item
        // ================================================================
        return std::nullopt; // placeholder
    }

    // Call when no more producers will push. Wakes all waiters.
    void close() {
        std::scoped_lock lock(mut_);
        closed_ = true;
        not_full_.notify_all();
        not_empty_.notify_all();
    }

    std::size_t capacity() const { return capacity_; }

private:
    std::size_t capacity_;
    std::deque<T> queue_;
    std::mutex mut_;
    std::condition_variable not_full_;
    std::condition_variable not_empty_;
    bool closed_ = false;
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
                (void)*item; // pretend to flush it
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
