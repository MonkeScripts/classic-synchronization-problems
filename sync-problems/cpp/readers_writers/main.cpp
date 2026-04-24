// Readers-Writers — starter template (C++20)
//
// Scenario: in-memory KV cache.
//   - `get(k)` is a READ — many concurrent readers OK.
//   - `set(k, v)` is a WRITE — exclusive.
//
// Three implementations to compare:
//   1. std::shared_mutex (library version — one line).
//   2. Hand-rolled with mutex + condvar (lecture Lightswitch pattern).
//   3. No-starve reader/writer using a turnstile (lecture slide 12).
//
// Invariants (checked by ActiveCounters below):
//   - active_writers <= 1
//   - (active_readers > 0)  implies  (active_writers == 0)
//   - (active_writers == 1) implies  (active_readers == 0)
//
// Measure:
//   - Reads/sec at varying reader:writer ratios (e.g. 10:1, 100:1).
//   - Did any writer starve (max wait time)?

#include <atomic>
#include <chrono>
#include <iostream>
#include <mutex>
#include <random>
#include <shared_mutex>
#include <string>
#include <thread>
#include <unordered_map>
#include <vector>
#include <semaphore>

using namespace std::chrono_literals;

constexpr int NUM_READERS = 8;
constexpr int NUM_WRITERS = 2;
constexpr int OPS_PER_THREAD = 1000;

// Runtime invariant tracker. Every impl wraps these around its critical sections.
struct ActiveCounters {
    std::atomic<int> readers{0};
    std::atomic<int> writers{0};

    void enter_read() {
        readers.fetch_add(1);
        if (writers.load() != 0) {
            std::cerr << "INVARIANT: reader entered with writers active\n";
            std::abort();
        }
    }
    void exit_read() { readers.fetch_sub(1); }

    void enter_write() {
        writers.fetch_add(1);
        if (writers.load() != 1 || readers.load() != 0) {
            std::cerr << "INVARIANT: writer entered with other writers or readers "
                         "(writers=" << writers.load() << " readers=" << readers.load() << ")\n";
            std::abort();
        }
    }
    void exit_write() { writers.fetch_sub(1); }
};

// ==========================================================================
// Implementation #1: shared_mutex (library). Keep as reference.
// ==========================================================================
class KVCacheShared {
public:
    std::string get(const std::string& k) {
        std::shared_lock lock(mu_);
        counters_.enter_read();
        auto it = map_.find(k);
        std::string result = (it != map_.end()) ? it->second : "";
        counters_.exit_read();
        return result;
    }
    void set(const std::string& k, const std::string& v) {
        std::unique_lock lock(mu_);
        counters_.enter_write();
        map_[k] = v;
        counters_.exit_write();
    }
    ActiveCounters counters_;
private:
    mutable std::shared_mutex mu_;
    std::unordered_map<std::string, std::string> map_;
};

// ==========================================================================
// Implementation #2: Hand-rolled. Fill this in.
// ==========================================================================
class KVCacheHandRolled {
public:
    std::string get(const std::string& k) {
        // TODO: acquire read lock (lightswitch pattern — first reader blocks writers,
        //       last reader releases)
        bibo_sem_.acquire();
        ++rc_;
        if (rc_ == 1) {
            roomEmptySem_.acquire();
        }
        bibo_sem_.release();        
        counters_.enter_read();
        auto it = map_.find(k);
        std::string result = (it != map_.end()) ? it->second : "";
        counters_.exit_read();
        bibo_sem_.acquire();
        --rc_;
        if (rc_ == 0) {
            roomEmptySem_.release();
        }
        bibo_sem_.release();
        // TODO: release read lock
        return result;
    }
    void set(const std::string& k, const std::string& v) {
        // TODO: acquire write lock
        roomEmptySem_.acquire();
        counters_.enter_write();
        map_[k] = v;
        counters_.exit_write();
        roomEmptySem_.release();
        // TODO: release write lock
    }
    ActiveCounters counters_;
private:
    // TODO: add mutex, counter, condition variable, roomEmpty flag, etc.
    std::unordered_map<std::string, std::string> map_;
    std::counting_semaphore<1> roomEmptySem_{1};
    std::counting_semaphore<1> bibo_sem_{1};
    std::atomic<int> rc_{0};

};

// ==========================================================================
// Implementation #3: No-starve reader/writer via a turnstile (lecture slide 12).
// ==========================================================================
// Problem with #2: reader-preference. Under a steady reader load, `rc_` never
// hits 0, so `roomEmptySem_` is never released, and writers wait forever.
// You can see it yourself by setting NUM_READERS=50 / NUM_WRITERS=1 (and
// optionally inserting a small sleep into the read critical section) and
// measuring per-write wait time.
//
// Fix: add a THIRD semaphore — the `turnstile` — that every thread must pass
// through before entering its acquisition path.
//   - WRITERS hold the turnstile for the entire critical section, *including*
//     the wait on roomEmpty. While a writer is queued or writing, the
//     turnstile is held, so new readers arriving get stopped there.
//   - READERS "gate through" the turnstile: acquire, then release immediately.
//     No reader holds the turnstile for any non-trivial time. Once past the
//     turnstile, readers follow the same Lightswitch pattern as #2.
//
// Consequence: once a writer is waiting on roomEmpty, no NEW readers can join
// the readers currently in the room. Existing readers finish, `rc_` drops to 0,
// the writer gets `roomEmpty`, writes, and releases the turnstile. Any readers
// that arrived during that window gate through and resume.
//
// Fairness caveat: std::counting_semaphore does NOT guarantee FIFO wakeup
// order — `release()` wakes *some* waiter, not necessarily the one that has
// been waiting longest. Strict starve-freedom depends on the underlying
// implementation's fairness (libstdc++/glibc uses futexes, which are
// approximately fair in practice but not strictly FIFO). Treat this
// implementation as "starve-free under reasonable fairness," which is enough
// for the comparison the exercise is asking for.
// ==========================================================================
class KVCacheNoStarve {
public:
    std::string get(const std::string& k) {
        turnstile_.acquire();
        turnstile_.release();
        bibo_sem_.acquire();
        ++rc_;
        if (rc_ == 1) roomEmptySem_.acquire();
        bibo_sem_.release();
        counters_.enter_read();
        auto it = map_.find(k);
        std::string result = (it != map_.end()) ? it->second : "";
        counters_.exit_read();
        bibo_sem_.acquire();
        --rc_;
        if (rc_ == 0) roomEmptySem_.release();
        bibo_sem_.release();
        return result;
    }
    void set(const std::string& k, const std::string& v) {
        turnstile_.acquire();
        roomEmptySem_.acquire();
        counters_.enter_write();
        map_[k] = v;
        counters_.exit_write();
        roomEmptySem_.release();
        turnstile_.release();
    }
    ActiveCounters counters_;
private:
    std::counting_semaphore<1> turnstile_{1};
    std::counting_semaphore<1> roomEmptySem_{1};
    std::counting_semaphore<1> bibo_sem_{1};
    int rc_ = 0;
    std::unordered_map<std::string, std::string> map_;
};

template <typename Cache>
void run_benchmark(const char* name) {
    Cache cache;
    std::atomic<long long> reads{0}, writes{0};
    auto start = std::chrono::steady_clock::now();

    std::vector<std::thread> threads;
    for (int r = 0; r < NUM_READERS; ++r) {
        threads.emplace_back([&, rid = r] {
            std::mt19937 rng(rid);
            for (int i = 0; i < OPS_PER_THREAD; ++i) {
                cache.get("key" + std::to_string(rng() % 10));
                reads.fetch_add(1);
            }
        });
    }
    for (int w = 0; w < NUM_WRITERS; ++w) {
        threads.emplace_back([&, wid = w] {
            std::mt19937 rng(wid + 1000);
            for (int i = 0; i < OPS_PER_THREAD; ++i) {
                cache.set("key" + std::to_string(rng() % 10),
                          "v" + std::to_string(i));
                writes.fetch_add(1);
            }
        });
    }
    for (auto& t : threads) t.join();

    auto elapsed = std::chrono::steady_clock::now() - start;
    auto ms = std::chrono::duration_cast<std::chrono::milliseconds>(elapsed).count();
    std::cout << name << ": " << ms << " ms  "
              << "reads=" << reads.load() << " writes=" << writes.load() << "\n";
}

int main() {
    run_benchmark<KVCacheShared>("shared_mutex ");
    run_benchmark<KVCacheHandRolled>("hand-rolled  ");
    run_benchmark<KVCacheNoStarve>("no-starve    ");
}
