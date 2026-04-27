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
#include <condition_variable>
#include <random>
#include <thread>
#include <vector>
#include <semaphore>

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
std::array<std::mutex, N> chopsticks;
std::counting_semaphore<> num_eaters{N-1};

// --- Tanenbaum strategy: per-philosopher state + one CV per philosopher ---
// algo_states is the algorithm's OWN state — separate from `states[]` (which
// is the invariant oracle, line 32). Same separation principle as the
// readers-writers exercise: keep the algorithm's source-of-truth distinct
// from the witness so the witness can catch bugs in the state machine.
std::array<State, N> algo_states{};                         // guarded by tanenbaum_mutex
std::mutex tanenbaum_mutex;
std::array<std::condition_variable, N> tanenbaum_cv{};


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

void eat_footman(std::size_t pid, std::mt19937& rng) {
    states[pid].store(State::HUNGRY);
    //limit the number of eaters -> One less eater to care about

    num_eaters.acquire();
    //still need locks on both chopsticks
    std::size_t left = pid;
    std::size_t right = (pid + 1) % N;
    // std::scoped_lock lk {chopsticks[left], chopsticks[right]}; REdundant
    // Alternative

    chopsticks[left].lock();
    chopsticks[right].lock();

    // ======================================================================
    // TODO: Acquire resources here according to your strategy.
    // ======================================================================

    states[pid].store(State::EATING);
    check_invariant(pid);
    meal_counts[pid].fetch_add(1);
    jitter(rng, 3);
    states[pid].store(State::THINKING);
    //conventional way to unlock
    chopsticks[right].unlock();
    chopsticks[left].unlock();


    num_eaters.release();

    // ======================================================================
    // TODO: Release resources here.
    // ======================================================================
}


// =========================================================================
// Tanenbaum: state-tracking strategy (NO chopstick locks at all)
// =========================================================================
// Idea:
//   take_forks(pid):  set HUNGRY, test self, then wait until I am EATING.
//   put_forks(pid):   set THINKING, test left, test right (give neighbors a chance).
//   test(pid):        if I'm HUNGRY and both neighbors !EATING,
//                     set me EATING and notify cv[pid].
//
// All algo_states reads + decisions + writes happen under tanenbaum_mutex.
// The mutex is RELEASED while eating — no neighbor can enter EATING anyway,
// because their test() will see algo_states[pid] == EATING and refuse.
//
// Property:  deadlock-free, but NOT starvation-free.
//   Two philosophers on either side of a hungry one can alternate forever
//   and starve the middle one. Watch the spread output to see this.

void tanenbaum_safe_to_eat(std::size_t pid) {
    // PRECONDITION: caller holds tanenbaum_mutex.
    std::size_t left  = (pid + N - 1) % N;
    std::size_t right = (pid + 1) % N;
    if (algo_states[pid] == State::HUNGRY &&
        algo_states[left] != State::EATING &&
        algo_states[right] != State::EATING) {
            algo_states[pid] = State::EATING;
            tanenbaum_cv[pid].notify_one(); // notify yourself
        }
    // TODO: if algo_states[pid]   == State::HUNGRY
    //       && algo_states[left]  != State::EATING
    //       && algo_states[right] != State::EATING:
    //         set algo_states[pid] = State::EATING
    //         tanenbaum_cv[pid].notify_one();
}

void eat_tanenbaum(std::size_t pid, std::mt19937& rng) {
    states[pid].store(State::HUNGRY);

    // ===== take_forks =====
    {
        std::unique_lock<std::mutex> lock(tanenbaum_mutex);
        algo_states[pid] = State::HUNGRY;
        tanenbaum_safe_to_eat(pid);
        tanenbaum_cv[pid].wait(lock, [pid] {
            return algo_states[pid] == State::EATING;
        });
        // TODO: algo_states[pid] = State::HUNGRY;
        // TODO: tanenbaum_safe_to_eat(pid);
        // TODO: predicate-loop wait on tanenbaum_cv[pid]
        //       until algo_states[pid] == State::EATING
        //       (use the lambda overload `wait(lock, pred)` or a manual while-loop)
    }
    // Mutex released. We're in algo-EATING; no neighbor can enter EATING
    // until we put_forks below.

    states[pid].store(State::EATING);
    check_invariant(pid);
    meal_counts[pid].fetch_add(1);
    jitter(rng, 3);
    states[pid].store(State::THINKING);

    // ===== put_forks =====
    {
        std::unique_lock<std::mutex> lock(tanenbaum_mutex);
        std::size_t left  = (pid + N - 1) % N;
        std::size_t right = (pid + 1) % N;
        algo_states[pid] = State::THINKING;
        tanenbaum_safe_to_eat(left);
        tanenbaum_safe_to_eat(right);
        // TODO: algo_states[pid] = State::THINKING;
        // TODO: tanenbaum_safe_to_eat(left);   // give left a chance to start
        // TODO: tanenbaum_safe_to_eat(right);  // give right a chance to start
    }
}


void eat_scoped_lock(std::size_t pid, std::mt19937& rng) {
    states[pid].store(State::HUNGRY);

    // ======================================================================
    // TODO: Acquire resources here according to your strategy.
    // ======================================================================
    std::size_t left = pid;
    std::size_t right = (pid + 1) % N;
    std::scoped_lock lk {chopsticks[left], chopsticks[right]}; // redundant
    states[pid].store(State::EATING);
    check_invariant(pid);
    meal_counts[pid].fetch_add(1);
    jitter(rng, 3);
    states[pid].store(State::THINKING);

    // ======================================================================
    // TODO: Release resources here.
    // ======================================================================
}


using EatFn = void(*)(std::size_t, std::mt19937&);

void philosopher(std::size_t pid, EatFn eat_fn) {
    std::mt19937 rng(static_cast<unsigned>(pid) ^ 0xC0FFEEu);
    for (int i = 0; i < MEALS_PER_PHILOSOPHER; ++i) {
        think(pid, rng);
        eat_fn(pid, rng);
    }
}

void run_strategy(const char* name, EatFn eat_fn) {
    // Reset all shared state between runs. Chopstick mutexes and num_eaters
    // are self-balancing (every acquire pairs with a release in a clean run).
    for (std::size_t i = 0; i < N; ++i) {
        states[i].store(State::THINKING);
        algo_states[i] = State::THINKING;
        meal_counts[i].store(0);
    }

    std::vector<std::thread> threads;
    threads.reserve(N);
    auto start = std::chrono::steady_clock::now();

    for (std::size_t i = 0; i < N; ++i) {
        threads.emplace_back(philosopher, i, eat_fn);
    }
    for (auto& t : threads) t.join();

    auto elapsed = std::chrono::steady_clock::now() - start;
    std::cout << "=== " << name << " ===\n";
    std::cout << "Done in "
              << std::chrono::duration_cast<std::chrono::milliseconds>(elapsed).count()
              << " ms\n";
    std::cout << "Meals per philosopher: ";
    for (std::size_t i = 0; i < N; ++i) std::cout << meal_counts[i].load() << " ";
    std::cout << "\n";

    int min_meals = meal_counts[0].load(), max_meals = meal_counts[0].load();
    for (std::size_t i = 1; i < N; ++i) {
        min_meals = std::min(min_meals, meal_counts[i].load());
        max_meals = std::max(max_meals, meal_counts[i].load());
    }
    std::cout << "min=" << min_meals << " max=" << max_meals
              << " spread=" << (max_meals - min_meals) << "\n\n";
}

int main() {
    run_strategy("scoped_lock", eat_scoped_lock);
    run_strategy("footman",     eat_footman);
    run_strategy("tanenbaum",   eat_tanenbaum);
}
