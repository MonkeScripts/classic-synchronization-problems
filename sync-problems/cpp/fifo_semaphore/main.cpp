// FIFO Semaphore — starter template (C++20)
//
// Scenario (lecture T7.2): a counting semaphore with the strict guarantee
// that waiters are unblocked in arrival order — first to call acquire() is
// first to wake when release() comes in. std::counting_semaphore does NOT
// guarantee this; you're implementing the FIFO primitive yourself.
//
// Build (from sync-problems/cpp/):
//   cmake -S . -B build && cmake --build build
//   ./build/fifo_semaphore/fifo_semaphore
//
// Strategies worth trying (lecture demos 4-5, Task 1):
//   1. Ticket queue (FIFOSemaphore2): atomic next_ticket + atomic now_serving
//      + spinwait. Each acquirer fetch_add()s next_ticket; release() fetch_add()s
//      now_serving. Block while now_serving < my_ticket. Spinwait keeps it
//      simple but burns CPU under contention. (Memory-order question: why does
//      release-order on now_serving + acquire-load suffice across multiple
//      releases? See lecture aside on extended release sequences.)
//   2. Same ticket queue, but replace the spinwait with a condition variable
//      (Task 1) — or std::atomic<T>::wait if you're feeling adventurous.
//   3. Queue of per-waiter binary semaphores (FIFOSemaphore5). Each waiter
//      pushes its own std::binary_semaphore onto a mutex-protected queue and
//      blocks on it. release() pops the front and signals exactly that one.
//
// Invariant checked by the harness:
//   - Threads are spawned in order 0..N_THREADS with ARRIVAL_SPACING_MS gaps.
//     Each sleeps then calls acquire(); the spacing makes the queue order
//     match the spawn order with high probability.
//   - After all threads finish, wake_indices[i] should equal i (FIFO).
//   - If your spacing is too tight relative to scheduler jitter, a passing
//     run can be a false positive — bump ARRIVAL_SPACING_MS or spawn more
//     threads to stress it.

#include <atomic>
#include <cassert>
#include <chrono>
#include <condition_variable>
#include <cstddef>
#include <iostream>
#include <mutex>
#include <queue>
#include <thread>
#include <vector>

using namespace std::chrono_literals;

constexpr int N_THREADS = 16;
constexpr std::ptrdiff_t INITIAL_COUNT = 0;
constexpr int ARRIVAL_SPACING_MS = 10;

// ==========================================================================
// TODO: Implement FifoSemaphore.
//
// API contract:
//   FifoSemaphore(initial_count)
//   acquire():  block until count > 0, then atomically count -= 1.
//               Waiters MUST wake in arrival order.
//   release():  count += 1; if any waiter is queued, the OLDEST one wakes.
// ==========================================================================
class FifoSemaphore {
public:
    explicit FifoSemaphore(std::ptrdiff_t initial_count) : now_serving{initial_count}{
        
    }

    void acquire() {
        // TODO: see strategy options at top of file.
        std::unique_lock<std::mutex> lk{mut_};
        int current_ticket = next_ticket.fetch_add(1);
        cv.wait(lk, [current_ticket, this] {
            return current_ticket <= now_serving;
        });

    }

    void release() {
        // TODO
        std::unique_lock<std::mutex> lk{mut_};
        now_serving.fetch_add(1);
        cv.notify_all(); // Need to notify all because we can have multiple acquires waiting for us. They can wakeup but we are protected by the predicate
    }

private:
    // TODO: fields
    std::atomic<std::ptrdiff_t> now_serving;
    std::atomic<std::ptrdiff_t> next_ticket{1};
    std::condition_variable cv;
    std::mutex mut_;
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

    // Wait until everyone has called acquire() and is queued.
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
