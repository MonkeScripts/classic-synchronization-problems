// FIFO Semaphore — queue-of-semaphores template (C++20)
//
// Companion to main.cpp (ticket-queue + condvar). Same problem, different
// strategy: instead of one shared cv that everyone waits on with a ticket
// number, give each waiter its OWN binary_semaphore and queue them. release()
// pops the front of the queue and signals exactly that one — no spurious
// wakeups, no shared cv, FIFO-by-construction. Mirrors lecture FIFOSemaphore5.
//
// Build (from sync-problems/cpp/):
//   cmake -S . -B build && cmake --build build
//   ./build/fifo_semaphore/fifo_semaphore_queue
//
// Why this is interesting:
//   - Each waiter blocks on its own private semaphore — there is no shared
//     wait set, so no thundering herd.
//   - The std::queue<shared_ptr<Waiter>> IS the FIFO order. release() simply
//     pops the front; the waiter at the front is, by construction, the
//     oldest arriver.
//   - The mutex protects the queue + count_; the binary_semaphore handles
//     the actual blocking.
//
// Cost vs ticket-queue version:
//   - One allocation per waiter (for the Waiter struct). Lecture aside
//     mentions an intrusive linked list as a way to avoid this; we'll skip
//     that optimization here for clarity.
//
// Invariant checked by the harness: same as main.cpp.

#include <atomic>
#include <chrono>
#include <cstddef>
#include <iostream>
#include <memory>
#include <mutex>
#include <queue>
#include <semaphore>
#include <thread>
#include <vector>

using namespace std::chrono_literals;

constexpr int N_THREADS = 16;
constexpr std::ptrdiff_t INITIAL_COUNT = 0;
constexpr int ARRIVAL_SPACING_MS = 10;

// ==========================================================================
// TODO: Implement FifoSemaphore using the QUEUE strategy.
//
// Suggested shape:
//
//   struct Waiter {
//       std::binary_semaphore sem{0};
//   };
//
//   class FifoSemaphore {
//   public:
//       explicit FifoSemaphore(std::ptrdiff_t initial_count) : count_{initial_count} {}
//
//       void acquire() {
//           auto waiter = std::make_shared<Waiter>();
//           {
//               std::scoped_lock lk{mut_};
//               if (count_ > 0) {
//                   count_--;
//                   return;          // permit available, take it without blocking
//               }
//               waiters_.push(waiter);  // no permit, queue myself
//           }
//           waiter->sem.acquire();   // block on MY semaphore (released outside the lock)
//       }
//
//       void release() {
//           std::shared_ptr<Waiter> waiter;
//           {
//               std::scoped_lock lk{mut_};
//               if (waiters_.empty()) {
//                   count_++;        // no waiters, just bump the count_
//                   return;
//               }
//               waiter = waiters_.front();
//               waiters_.pop();
//           }
//           waiter->sem.release();   // wake the front of the queue
//       }
//
//   private:
//       std::mutex mut_;
//       std::queue<std::shared_ptr<Waiter>> waiters_;
//       std::ptrdiff_t count_;
//   };
//
// Notes:
//   - waiter->sem.acquire()/release() happens OUTSIDE the lock, intentionally.
//     The lock only protects the queue + count_. Holding the lock while parking
//     on the semaphore would deadlock.
//   - Why shared_ptr<Waiter>? Because the waiter struct lives in the queue
//     beyond the acquirer's stack frame; the lecture aside discusses an
//     intrusive linked list alternative (stack-allocated Waiters), but it's
//     not portable — see the aside on use-after-free hazards.
// ==========================================================================
struct Waiter {
    std::binary_semaphore sem{0};
};

class FifoSemaphore {
public:
    explicit FifoSemaphore(std::ptrdiff_t initial_count) : count_{initial_count} {
    }

    void acquire() {
        // TODO
        auto waiter = std::make_shared<Waiter>();
        {
            std::unique_lock<std::mutex> lk{mut_};
            // consume permits
            if ((count_ > 0)) {
                --count_;
                return;
            }
            // Make a new waiter and add to queue
            waiters_.push(waiter);
        }
        waiter->sem.acquire();        
    }

    void release() {
        // TODO
        std::shared_ptr<Waiter> waiter;
        {
            std::unique_lock<std::mutex> lk{mut_};
            if (waiters_.empty()) {
                // Add permit
                ++count_;
                return;
            }

            // pop from queue and release
            waiter = waiters_.front();
            waiters_.pop();
        }
        waiter->sem.release();
    }

private:
    // TODO: fields
    std::mutex mut_;
    std::queue<std::shared_ptr<Waiter>> waiters_;
    std::ptrdiff_t count_;

};

// ----- Invariant tracking (do not touch) -----
std::atomic<int> global_wake_counter{0};

int main() {
    FifoSemaphore sem(INITIAL_COUNT);
    std::vector<std::thread> threads;
    std::vector<std::atomic<int>> wake_indices(N_THREADS);
    for (auto& w : wake_indices) w.store(-1);

    auto start = std::chrono::steady_clock::now();
    for (int i = 0; i < N_THREADS; ++i) {
        threads.emplace_back([i, &sem, &wake_indices] {
            std::this_thread::sleep_for(std::chrono::milliseconds(i * ARRIVAL_SPACING_MS));
            sem.acquire();
            int wake = global_wake_counter.fetch_add(1);
            wake_indices[i].store(wake);
            std::this_thread::sleep_for(2ms);
            sem.release();
        });
    }

    std::this_thread::sleep_for(std::chrono::milliseconds((N_THREADS + 2) * ARRIVAL_SPACING_MS));
    sem.release(); // kick off the chain

    for (auto& t : threads) t.join();
    auto elapsed = std::chrono::steady_clock::now() - start;

    int violations = 0;
    for (int i = 0; i < N_THREADS; ++i) {
        int w = wake_indices[i].load();
        if (w != i) {
            std::cerr << "FIFO violation: thread " << i
                      << " woke at index " << w << "\n";
            violations++;
        }
    }
    std::cout << "Took "
              << std::chrono::duration_cast<std::chrono::milliseconds>(elapsed).count()
              << "ms, " << violations << " FIFO violations\n";
    return violations == 0 ? 0 : 1;
}
