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
        counters_.enter_read();
        auto it = map_.find(k);
        std::string result = (it != map_.end()) ? it->second : "";
        counters_.exit_read();
        // TODO: release read lock
        return result;
    }
    void set(const std::string& k, const std::string& v) {
        // TODO: acquire write lock
        counters_.enter_write();
        map_[k] = v;
        counters_.exit_write();
        // TODO: release write lock
    }
    ActiveCounters counters_;
private:
    // TODO: add mutex, counter, condition variable, roomEmpty flag, etc.
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
}
