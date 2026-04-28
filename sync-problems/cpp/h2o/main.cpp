// H2O Water Factory — starter template (C++20)
//
// Scenario (lecture T7.1): hydrogen and oxygen atoms arrive at a factory.
// When 2 H + 1 O are present, they bond into a water molecule.
//
// Build (from sync-problems/cpp/):
//   cmake -S . -B build && cmake --build build
//   ./build/h2o/h2o
//
// Strategies worth trying (lecture):
//   1. WaterFactory1 (BUGGY): just std::barrier<>{3}. Catches the count
//      but NOT the type — 3 oxygens can bond into ozone.
//   2. WaterFactory2 (BUGGY): std::counting_semaphore<>{2}/{1} for H/O.
//      Catches the type but lets atoms bond solo without 3 present.
//   3. WaterFactory3 (CORRECT): combine semaphores AND barrier — semaphores
//      gate the type counts; barrier gates the start of bonding.
//
// Invariants checked by the harness:
//   - h_in_bond <= 2 always (catches ozone)
//   - o_in_bond <= 1 always (catches double-oxygen)
//   - max_total_in_bond should reach 3 (catches "no barrier" — atoms bonding solo)
//   - h_total_bonds == 2 * N_MOLECULES, o_total_bonds == N_MOLECULES

#include <atomic>
#include <cassert>
#include <chrono>
#include <iostream>
#include <random>
#include <thread>
#include <vector>
#include <semaphore>
#include <barrier>
using namespace std::chrono_literals;

constexpr int N_MOLECULES = 50;
constexpr int N_HYDROGEN = 2 * N_MOLECULES;
constexpr int N_OXYGEN = N_MOLECULES;

// ----- Invariant tracking (do not touch) -----
std::atomic<int> h_in_bond{0};
std::atomic<int> o_in_bond{0};
std::atomic<int> max_total_in_bond{0};
std::atomic<int> h_total_bonds{0};
std::atomic<int> o_total_bonds{0};

void update_max_total() {
    int total = h_in_bond.load() + o_in_bond.load();
    int cur = max_total_in_bond.load();
    while (total > cur && !max_total_in_bond.compare_exchange_weak(cur, total)) {}
}

void bond_h(int id) {
    int h = h_in_bond.fetch_add(1) + 1;
    assert(h <= 2 && "ozone! more than 2 H atoms in bond simultaneously");
    update_max_total();
    std::this_thread::sleep_for(2ms);
    update_max_total();
    std::this_thread::sleep_for(1ms);
    h_in_bond.fetch_sub(1);
    h_total_bonds.fetch_add(1);
    (void)id;
}

void bond_o(int id) {
    int o = o_in_bond.fetch_add(1) + 1;
    assert(o <= 1 && "more than 1 O in bond simultaneously");
    update_max_total();
    std::this_thread::sleep_for(2ms);
    update_max_total();
    std::this_thread::sleep_for(1ms);
    o_in_bond.fetch_sub(1);
    o_total_bonds.fetch_add(1);
    (void)id;
}

// ==========================================================================
// TODO: Implement WaterFactory.
//
// API contract:
//   - hydrogen(id): block until grouped with 1 H + 1 O, then call bond_h(id).
//   - oxygen(id):   block until grouped with 2 H,         then call bond_o(id).
//
// Strategies (pick one, then try a second for comparison):
//   1. std::counting_semaphore<>{2} (H) + std::counting_semaphore<>{1} (O)
//      + std::barrier<>{3}. Each acquirer holds its semaphore permit ACROSS
//      the barrier+bond, then releases on exit. Mirrors WaterFactory3.
//   2. std::mutex + std::condition_variable + state machine. Wake the right
//      type when the right counts (h_waiting >= 2 && o_waiting >= 1) are met.
//   3. (Compare to) std::latch<3>: like barrier but single-use. What goes wrong
//      if you use a latch here? (hint: only one molecule can ever bond.)
// ==========================================================================
class WaterFactory {
public:
    void hydrogen(int id) {
        // TODO: coordinate, then bond_h(id)
        hydrogen_sem_.acquire();
        barrier.arrive_and_wait();
        bond_h(id);
        hydrogen_sem_.release();
    }

    void oxygen(int id) {
        // TODO: coordinate, then bond_o(id)
        oxygen_sem_.acquire();
        barrier.arrive_and_wait();
        bond_o(id);
        oxygen_sem_.release();
    }

private:
    // TODO: fields
    std::counting_semaphore<> oxygen_sem_{1};
    std::counting_semaphore<> hydrogen_sem_{2};
    std::barrier<> barrier{3};
};

int main() {
    WaterFactory factory;
    std::vector<std::thread> threads;
    threads.reserve(N_HYDROGEN + N_OXYGEN);

    std::mt19937 rng(42);
    std::uniform_int_distribution<int> jitter(0, 5);

    auto start = std::chrono::steady_clock::now();
    for (int i = 0; i < N_HYDROGEN; ++i) {
        std::this_thread::sleep_for(std::chrono::milliseconds(jitter(rng)));
        threads.emplace_back([&factory, i] { factory.hydrogen(i); });
    }
    for (int i = 0; i < N_OXYGEN; ++i) {
        std::this_thread::sleep_for(std::chrono::milliseconds(jitter(rng)));
        threads.emplace_back([&factory, i] { factory.oxygen(i); });
    }
    for (auto& t : threads) t.join();
    auto elapsed = std::chrono::steady_clock::now() - start;

    std::cout << "H bonds: " << h_total_bonds.load() << "/" << N_HYDROGEN
              << "  O bonds: " << o_total_bonds.load() << "/" << N_OXYGEN
              << "  max simultaneous in bond: " << max_total_in_bond.load()
              << "  time: "
              << std::chrono::duration_cast<std::chrono::milliseconds>(elapsed).count()
              << "ms\n";

    bool ok = true;
    if (h_total_bonds.load() != N_HYDROGEN) {
        std::cerr << "MISSING H BONDS!\n"; ok = false;
    }
    if (o_total_bonds.load() != N_OXYGEN) {
        std::cerr << "MISSING O BONDS!\n"; ok = false;
    }
    if (max_total_in_bond.load() != 3) {
        std::cerr << "max_total_in_bond should be 3, got "
                  << max_total_in_bond.load()
                  << " — atoms are bonding without all 3 being present\n";
        ok = false;
    }
    return ok ? 0 : 1;
}
