// Search-Insert-Delete — Tier 2 Q8 (Allen Downey, Little Book of Semaphores 6.1.2)
//
// Three roles share a list:
//   - Searchers : may run concurrently with each other and with one inserter.
//   - Inserters : at most one at a time; may run alongside any number of searchers.
//   - Deleters  : exclusive — no searchers, no inserters concurrently.
//
// Two implementations:
//   1. Lightswitch (reader-preference style) — deleters can starve under load.
//   2. No-starve  — adds a turnstile so deleters cannot be locked out.
//
// Invariants (checked by ActiveCounters):
//   - 0 <= searchers
//   - 0 <= inserters <= 1
//   - deleters > 0  =>  searchers == 0 && inserters == 0

#include <atomic>
#include <chrono>
#include <iostream>
#include <mutex>
#include <random>
#include <semaphore>
#include <thread>
#include <vector>

using namespace std::chrono_literals;

constexpr int NUM_SEARCHERS  = 8;
constexpr int NUM_INSERTERS  = 3;
constexpr int NUM_DELETERS   = 2;
constexpr int OPS_PER_THREAD = 2000;
constexpr size_t MAX_SLOTS   = 1 << 14;   // pre-allocated capacity; never grows.

// Why a fixed-capacity vector + atomic size, instead of std::list / std::vector
// push_back? Downey's abstraction (searcher + inserter may overlap) requires the
// underlying data structure to publish new elements with release/acquire to a
// concurrent reader. std::list::push_back writes a node's `next` pointer with
// no happens-before edge to an iterating searcher — TSan flags it as a race,
// because it really IS a race per the C++ memory model, even on x86 where the
// observable outcome is benign. The vector + atomic_size_release/acquire pair
// is the smallest fix that honors the abstraction.

struct ActiveCounters {
    std::atomic<int> searchers{0};
    std::atomic<int> inserters{0};
    std::atomic<int> deleters{0};

    void enter_search() {
        searchers.fetch_add(1);
        if (deleters.load() != 0) {
            std::cerr << "INVARIANT: searcher entered with deleter active\n";
            std::abort();
        }
    }
    void exit_search() { searchers.fetch_sub(1); }

    void enter_insert() {
        int now = inserters.fetch_add(1) + 1;
        if (now > 1 || deleters.load() != 0) {
            std::cerr << "INVARIANT: inserter overlapped (inserters=" << now
                      << " deleters=" << deleters.load() << ")\n";
            std::abort();
        }
    }
    void exit_insert() { inserters.fetch_sub(1); }

    void enter_delete() {
        deleters.fetch_add(1);
        if (deleters.load() != 1 || searchers.load() != 0 || inserters.load() != 0) {
            std::cerr << "INVARIANT: deleter entered with others active "
                         "(deleters=" << deleters.load()
                      << " searchers=" << searchers.load()
                      << " inserters=" << inserters.load() << ")\n";
            std::abort();
        }
    }
    void exit_delete() { deleters.fetch_sub(1); }
};

// ==========================================================================
// Implementation #1: lightswitch (reader-preference). Deleters can starve.
//
// State:
//   mtx            — guards searcher_count
//   searcher_count — int
//   no_searcher    — binary sem, held while >=1 searcher is in the room
//   no_inserter    — binary sem, held while an inserter is in the room
//
// Lock order for deleter: no_searcher -> no_inserter (release in reverse).
// ==========================================================================
class SidListLightswitch {
public:
    SidListLightswitch() : data_(MAX_SLOTS) {}

    void search(int /*x*/) {
        mtx_.lock();
        ++searcher_count_;
        if (searcher_count_ == 1) no_searcher_.acquire();
        mtx_.unlock();

        counters_.enter_search();
        size_t n = size_.load(std::memory_order_acquire);
        long sink = 0;
        for (size_t i = 0; i < n; ++i) sink += data_[i];
        asm volatile("" : : "r"(sink) : "memory");
        counters_.exit_search();

        mtx_.lock();
        --searcher_count_;
        if (searcher_count_ == 0) no_searcher_.release();
        mtx_.unlock();
    }

    void insert(int x) {
        no_inserter_.acquire();
        counters_.enter_insert();
        size_t i = size_.load(std::memory_order_relaxed);
        if (i < MAX_SLOTS) {
            data_[i] = x;
            size_.store(i + 1, std::memory_order_release);   // publish slot i
        }
        counters_.exit_insert();
        no_inserter_.release();
    }

    void delete_one() {
        no_searcher_.acquire();
        no_inserter_.acquire();
        counters_.enter_delete();
        size_t i = size_.load(std::memory_order_relaxed);
        if (i > 0) size_.store(i - 1, std::memory_order_relaxed);   // pop back
        counters_.exit_delete();
        no_inserter_.release();
        no_searcher_.release();
    }

    ActiveCounters counters_;

private:
    std::mutex mtx_;
    int searcher_count_ = 0;
    std::counting_semaphore<1> no_searcher_{1};
    std::counting_semaphore<1> no_inserter_{1};
    std::vector<int> data_;
    std::atomic<size_t> size_{0};
};

// ==========================================================================
// Implementation #2: no-starve. A turnstile every entrant gates through;
// the deleter holds it across its whole acquisition path so new searchers /
// inserters arriving after a deleter is queued cannot leapfrog ahead.
//
// Caveat: std::counting_semaphore does not guarantee FIFO wakeup, so this
// is "starve-free under reasonable fairness" — same caveat as KVCacheNoStarve.
// ==========================================================================
class SidListNoStarve {
public:
    SidListNoStarve() : data_(MAX_SLOTS) {}

    void search(int /*x*/) {
        turnstile_.acquire();
        turnstile_.release();

        mtx_.lock();
        ++searcher_count_;
        if (searcher_count_ == 1) no_searcher_.acquire();
        mtx_.unlock();

        counters_.enter_search();
        size_t n = size_.load(std::memory_order_acquire);
        long sink = 0;
        for (size_t i = 0; i < n; ++i) sink += data_[i];
        asm volatile("" : : "r"(sink) : "memory");
        counters_.exit_search();

        mtx_.lock();
        --searcher_count_;
        if (searcher_count_ == 0) no_searcher_.release();
        mtx_.unlock();
    }

    void insert(int x) {
        turnstile_.acquire();
        turnstile_.release();

        no_inserter_.acquire();
        counters_.enter_insert();
        size_t i = size_.load(std::memory_order_relaxed);
        if (i < MAX_SLOTS) {
            data_[i] = x;
            size_.store(i + 1, std::memory_order_release);
        }
        counters_.exit_insert();
        no_inserter_.release();
    }

    void delete_one() {
        turnstile_.acquire();
        no_searcher_.acquire();
        no_inserter_.acquire();
        counters_.enter_delete();
        size_t i = size_.load(std::memory_order_relaxed);
        if (i > 0) size_.store(i - 1, std::memory_order_relaxed);
        counters_.exit_delete();
        no_inserter_.release();
        no_searcher_.release();
        turnstile_.release();
    }

    ActiveCounters counters_;

private:
    std::counting_semaphore<1> turnstile_{1};
    std::mutex mtx_;
    int searcher_count_ = 0;
    std::counting_semaphore<1> no_searcher_{1};
    std::counting_semaphore<1> no_inserter_{1};
    std::vector<int> data_;
    std::atomic<size_t> size_{0};
};

template <typename List>
void run_benchmark(const char* name) {
    List list;
    auto start = std::chrono::steady_clock::now();

    std::vector<std::thread> threads;
    std::atomic<long long> max_delete_wait_ns{0};

    for (int s = 0; s < NUM_SEARCHERS; ++s) {
        threads.emplace_back([&, sid = s] {
            std::mt19937 rng(sid);
            for (int i = 0; i < OPS_PER_THREAD; ++i) {
                list.search(rng() % 100);
            }
        });
    }
    for (int ins = 0; ins < NUM_INSERTERS; ++ins) {
        threads.emplace_back([&, iid = ins] {
            std::mt19937 rng(iid + 1000);
            for (int i = 0; i < OPS_PER_THREAD; ++i) {
                list.insert(static_cast<int>(rng() % 1000));
            }
        });
    }
    for (int d = 0; d < NUM_DELETERS; ++d) {
        threads.emplace_back([&] {
            for (int i = 0; i < OPS_PER_THREAD; ++i) {
                auto t0 = std::chrono::steady_clock::now();
                list.delete_one();
                auto wait = std::chrono::steady_clock::now() - t0;
                long long ns = std::chrono::duration_cast<std::chrono::nanoseconds>(wait).count();
                long long prev = max_delete_wait_ns.load();
                while (ns > prev && !max_delete_wait_ns.compare_exchange_weak(prev, ns)) {}
            }
        });
    }
    for (auto& t : threads) t.join();

    auto elapsed = std::chrono::steady_clock::now() - start;
    auto ms = std::chrono::duration_cast<std::chrono::milliseconds>(elapsed).count();
    std::cout << name << ": " << ms << " ms"
              << "  max_delete_wait=" << (max_delete_wait_ns.load() / 1000) << " us\n";
}

int main() {
    run_benchmark<SidListLightswitch>("lightswitch ");
    run_benchmark<SidListNoStarve>   ("no-starve   ");
}
