// Dining Philosophers — starter template (C++20)
//
// Your job: fill in `eat()` using your chosen strategy.
// Try at least TWO of these in separate branches / files:
//   1. Naive (demonstrate deadlock — then fix)
//   2. Asymmetric: last philosopher grabs right-first
//   3. std::scoped_lock (library does deadlock avoidance)
//   4. Footman semaphore (at most N-1 can try)
//   5. Tanenbaum state-tracking (per-philosopher condition)
//
// Invariant helper checks adjacency — USE IT. Call check_invariant()
// inside eat() while holding both chopsticks.

#include <array>
#include <atomic>
#include <chrono>
#include <iostream>
#include <mutex>
#include <random>
#include <thread>
#include <vector>

using namespace std::chrono_literals;

constexpr std::size_t N = 5;
constexpr int MEALS_PER_PHILOSOPHER = 50;

enum class State { THINKING, HUNGRY, EATING };

// Shared state for invariant checking only (not part of your solution!)
std::array<std::atomic<State>, N> states{};
std::array<std::atomic<int>, N> meal_counts{};

// Jitter helper — sprinkle over think/eat to expose races.
void jitter(std::mt19937& rng, int max_ms = 5) {
    std::uniform_int_distribution<int> d(0, max_ms);
    std::this_thread::sleep_for(std::chrono::milliseconds(d(rng)));
}

// Call this while in EATING state to catch adjacency violations.
void check_invariant(std::size_t pid) {
    std::size_t left = (pid + N - 1) % N;
    std::size_t right = (pid + 1) % N;
    if (states[left].load() == State::EATING) {
        std::cerr << "INVARIANT VIOLATED: philosopher " << pid
                  << " eating while left neighbor " << left << " is eating\n";
        std::abort();
    }
    if (states[right].load() == State::EATING) {
        std::cerr << "INVARIANT VIOLATED: philosopher " << pid
                  << " eating while right neighbor " << right << " is eating\n";
        std::abort();
    }
}

// ==========================================================================
// TODO: Declare your chopsticks / semaphores / whatever your strategy needs.
// ==========================================================================
// Example shape for strategy 3 (std::scoped_lock):
// std::array<std::mutex, N> chopsticks;

void think(std::size_t pid, std::mt19937& rng) {
    states[pid].store(State::THINKING);
    jitter(rng, 3);
}

void eat(std::size_t pid, std::mt19937& rng) {
    states[pid].store(State::HUNGRY);

    // ======================================================================
    // TODO: Acquire resources here according to your strategy.
    // ======================================================================

    states[pid].store(State::EATING);
    check_invariant(pid);
    meal_counts[pid].fetch_add(1);
    jitter(rng, 3);
    states[pid].store(State::THINKING);

    // ======================================================================
    // TODO: Release resources here.
    // ======================================================================
}

void philosopher(std::size_t pid) {
    std::mt19937 rng(static_cast<unsigned>(pid) ^ 0xC0FFEEu);
    for (int i = 0; i < MEALS_PER_PHILOSOPHER; ++i) {
        think(pid, rng);
        eat(pid, rng);
    }
}

int main() {
    std::vector<std::thread> threads;
    threads.reserve(N);
    auto start = std::chrono::steady_clock::now();

    for (std::size_t i = 0; i < N; ++i) {
        threads.emplace_back(philosopher, i);
    }
    for (auto& t : threads) t.join();

    auto elapsed = std::chrono::steady_clock::now() - start;
    std::cout << "Done in "
              << std::chrono::duration_cast<std::chrono::milliseconds>(elapsed).count()
              << " ms\n";
    std::cout << "Meals per philosopher: ";
    for (std::size_t i = 0; i < N; ++i) std::cout << meal_counts[i].load() << " ";
    std::cout << "\n";

    // Fairness check — if any philosopher ate far less, suspect starvation.
    int min_meals = meal_counts[0].load(), max_meals = meal_counts[0].load();
    for (std::size_t i = 1; i < N; ++i) {
        min_meals = std::min(min_meals, meal_counts[i].load());
        max_meals = std::max(max_meals, meal_counts[i].load());
    }
    std::cout << "min=" << min_meals << " max=" << max_meals
              << " spread=" << (max_meals - min_meals) << "\n";
}
