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
#include <cstdint>
#include <iostream>
#include <mutex>
#include <semaphore>
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
        {
            std::scoped_lock lock{mut_};
            count_++;
            if (count_ == expected_) {
                turnstile_.release();
                turnstile2_.acquire();
            }
        }
        // Domino entry
        turnstile_.acquire();
        turnstile_.release();
        {
            std::scoped_lock lock{mut_};
            count_ --;
            if (count_ == 0) {
                turnstile2_.release();
                turnstile_.acquire(); // Do not want new things to come in
            }
        }
        //Domino exit
        turnstile2_.acquire();
        turnstile2_.release();
    }

private:
    int expected_;
    int count_{0};
    std::counting_semaphore<1> turnstile_{0};
    std::counting_semaphore<1> turnstile2_{1}; //Start off with it being open
    std::mutex mut_;
    // TODO: mutex, condition_variable, counter, generation/round counter, etc.
};

// ==========================================================================
// TODO: Preloaded-turnstile variant (lecture slide 22).
//
// Instead of the domino (last thread release()s ONE token and each thread
// acquire/release to pass it along), the last thread release()s N tokens
// in one shot and each thread acquires exactly ONCE. Less semaphore traffic.
//
// Pattern:
//   Phase 1 — GATHER:
//     lock; count++; if (count == expected) turnstile1.release(expected);
//     unlock; turnstile1.acquire();
//
//   Phase 2 — DISPERSE:
//     lock; count--; if (count == 0) turnstile2.release(expected);
//     unlock; turnstile2.acquire();
//
// Why reusable "for free": release(N) puts N tokens in, all N threads each
// acquire one, the semaphore naturally ends at 0 — no explicit drain step
// (unlike the domino, which needs the line-58 turnstile_.acquire() to stop
// the leftover token from leaking into the next round).
//
// Pitfalls to think about before you implement:
//   (a) Why counting_semaphore<N_THREADS> and NOT counting_semaphore<1>?
//       (hint: release(N) past the max is UB.)
//   (b) Can a fast thread finish phase 2 of round R and loop into phase 1
//       of round R+1 before a slow thread acquires its phase-2 token?
//       Does that break anything? (Walk through it.)
// ==========================================================================
class MyBarrierPreloaded {
public:
    explicit MyBarrierPreloaded(int expected) : expected_(expected) {}

    void arrive_and_wait() {
        // Phase 1 — GATHER
        {
            std::scoped_lock lock{mut_};
            count_++;
            if (count_ == expected_) {
                turnstile1_.release(expected_);
            }
        }
        turnstile1_.acquire();

        // Phase 2 — DISPERSE
        {
            std::scoped_lock lock{mut_};
            count_--;
            if (count_ == 0) {
                turnstile2_.release(expected_);
            }
        }
        turnstile2_.acquire();
    }

private:
    int expected_;
    int count_{0};
    std::mutex mut_;
    std::counting_semaphore<N_THREADS> turnstile1_{0};
    std::counting_semaphore<N_THREADS> turnstile2_{0};
};

// ==========================================================================
// Mutex + condition_variable + generation counter (the "classic" reusable
// barrier — same shape as the Go cond version in go/barrier_cond/).
//
// Each thread snapshots `gen` on entry and waits while gen == generation_.
// The last arriver resets count, bumps generation, and notify_all()s.
// The predicate-loop in cv.wait(lock, pred) handles spurious wakeups for free.
// ==========================================================================
class MyBarrierCond {
public:
    explicit MyBarrierCond(int expected) : expected_(expected) {}

    void arrive_and_wait() {
        std::unique_lock<std::mutex> lock(mu_);

        const std::uint64_t gen = generation_;
        ++count_;

        if (count_ == expected_) {
            // Last one in: trip the barrier and start a new generation.
            count_ = 0;
            ++generation_;
            cv_.notify_all();
            return;
        }

        cv_.wait(lock, [this, gen] { return gen != generation_; });
    }

    // Non-copyable, non-movable: the mutex and cv aren't movable anyway.
    MyBarrierCond(const MyBarrierCond&) = delete;
    MyBarrierCond& operator=(const MyBarrierCond&) = delete;

private:
    std::mutex mu_;
    std::condition_variable cv_;
    int expected_;
    int count_ = 0;
    std::uint64_t generation_ = 0;
};

// Invariant checking: every thread announces the round it's about to enter
// AFTER the barrier. If any two threads disagree on round, something raced.
std::atomic<int> global_round{0};   // the round currently "in progress"
std::atomic<int> arrived_this_round{0};

void worker(int tid, MyBarrierCond& barrier) {
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
    MyBarrierCond barrier(N_THREADS);
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
