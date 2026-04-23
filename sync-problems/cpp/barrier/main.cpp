// Barrier — starter template (C++20)
//
// Your job: implement a REUSABLE barrier. Three attempts worth trying:
//   1. Naive counter + condition variable (works, simple).
//   2. Two-turnstile semaphore (lecture slides 20-21).
//   3. Preloaded turnstile (lecture slide 22) — release(expected) to pass many at once.
//   4. (Compare to) std::barrier — the standard library's version.
//
// Invariant: when any thread is in round R+1, NO thread is still in round R.
// The `round` atomic + assert below checks this across phases.
//
// Scenario used: N threads each do WORK_PER_ROUND iterations of dummy "work"
// separated by barrier syncs. If you implement it wrong, some thread will
// race ahead and the assert will fire.

#include <atomic>
#include <chrono>
#include <condition_variable>
#include <iostream>
#include <mutex>
#include <thread>
#include <vector>

using namespace std::chrono_literals;

constexpr int N_THREADS = 8;
constexpr int ROUNDS = 100;

// ==========================================================================
// TODO: Replace this with YOUR barrier implementation.
// This placeholder is obviously wrong (it does nothing) — replace it.
// ==========================================================================
class MyBarrier {
public:
    explicit MyBarrier(int expected) : expected_(expected) {
        (void)expected_; // silence unused warning until you use it
    }

    void arrive_and_wait() {
        // TODO: block until `expected` threads have called this,
        // then release all of them. Make it REUSABLE (correct across rounds).
    }

private:
    int expected_;
    // TODO: mutex, condition_variable, counter, generation/round counter, etc.
};

// Invariant checking: every thread announces the round it's about to enter
// AFTER the barrier. If any two threads disagree on round, something raced.
std::atomic<int> global_round{0};   // the round currently "in progress"
std::atomic<int> arrived_this_round{0};

void worker(int tid, MyBarrier& barrier) {
    for (int r = 0; r < ROUNDS; ++r) {
        // --- Phase 1: do some work for round r ---
        // (Pretend work — volatile prevents over-optimizing)
        volatile int x = 0;
        for (int i = 0; i < 1000; ++i) x += i;

        // --- Barrier ---
        barrier.arrive_and_wait();

        // --- Invariant check: we should now all agree we just finished round r ---
        int expected_round = r;
        int seen = global_round.load();
        if (seen != expected_round) {
            std::cerr << "THREAD " << tid << " saw round " << seen
                      << " but expected " << expected_round << "\n";
            std::abort();
        }
        int arrived = arrived_this_round.fetch_add(1) + 1;
        if (arrived == N_THREADS) {
            // Last thread advances the round and resets.
            arrived_this_round.store(0);
            global_round.store(r + 1);
        }

        // --- Barrier again to keep everyone in lockstep before next round ---
        barrier.arrive_and_wait();
    }
}

int main() {
    MyBarrier barrier(N_THREADS);
    std::vector<std::thread> threads;
    auto start = std::chrono::steady_clock::now();
    for (int t = 0; t < N_THREADS; ++t) {
        threads.emplace_back(worker, t, std::ref(barrier));
    }
    for (auto& t : threads) t.join();
    auto elapsed = std::chrono::steady_clock::now() - start;
    std::cout << "Completed " << ROUNDS << " rounds with " << N_THREADS
              << " threads in "
              << std::chrono::duration_cast<std::chrono::milliseconds>(elapsed).count()
              << " ms\n";
}
