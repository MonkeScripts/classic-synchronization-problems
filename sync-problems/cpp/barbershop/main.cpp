// Barbershop — starter template (C++20)
//
// Scenario: food-truck line with CHAIRS waiting slots.
//   - TOTAL_CUSTOMERS customers arrive over time (Poisson-ish).
//   - If all chairs are full, customer balks and goes home.
//   - Barber sleeps when no customers, wakes when one arrives.
//
// Your job: implement `Barbershop::customer()` and `Barbershop::barber()`
// using the 4 semaphores from lecture (slide 44):
//   mutex, customer, barber, customerDone, barberDone.
//
// Invariants to enforce + check:
//   (A) counter `customers` always matches actual number of customers inside shop
//   (B) no customer "lost" — every customer either got a haircut or balked
//   (C) served count + balked count == TOTAL_CUSTOMERS
//
// Stretch: FIFO variant. Then multiple-barbers variant.

#include <atomic>
#include <chrono>
#include <iostream>
#include <mutex>
#include <random>
#include <semaphore>
#include <thread>
#include <vector>

using namespace std::chrono_literals;

constexpr std::size_t CHAIRS = 3;
constexpr int TOTAL_CUSTOMERS = 30;

class Barbershop {
public:
    // Returns true if served, false if balked.
    bool customer(int customer_id) {
        (void)customer_id;
        // ==========================================================
        // TODO:
        //   - acquire mutex
        //   - if customers_ == CHAIRS: release mutex, return false (balked)
        //   - customers_ += 1
        //   - release mutex
        //   - signal `customer_sem_` (wake barber / enqueue self)
        //   - wait on `barber_sem_` (my turn)
        //   - (get haircut happens here — simulate with sleep)
        //   - signal `customer_done_sem_`
        //   - wait on `barber_done_sem_`
        //   - acquire mutex, customers_ -= 1, release mutex
        //   - return true
        // ==========================================================
        return false; // placeholder
    }

    void barber() {
        while (!stop_.load()) {
            // ==========================================================
            // TODO:
            //   - wait on customer_sem_  (or check stop_ flag)
            //   - signal barber_sem_
            //   - cut hair (sleep)
            //   - wait on customer_done_sem_
            //   - signal barber_done_sem_
            // ==========================================================
            break; // placeholder — remove when implemented
        }
    }

    void stop() { stop_.store(true); customer_sem_.release(); }

private:
    std::size_t customers_ = 0;
    std::mutex mut_;
    std::counting_semaphore<> customer_sem_{0};
    std::counting_semaphore<> barber_sem_{0};
    std::counting_semaphore<> customer_done_sem_{0};
    std::counting_semaphore<> barber_done_sem_{0};
    std::atomic<bool> stop_{false};
};

int main() {
    Barbershop shop;
    std::thread barber_thread([&] { shop.barber(); });

    std::atomic<int> served{0};
    std::atomic<int> balked{0};

    std::vector<std::thread> customers;
    std::mt19937 rng(42);
    std::exponential_distribution<double> arrival(1.0 / 20.0); // mean 20ms between arrivals

    for (int i = 0; i < TOTAL_CUSTOMERS; ++i) {
        auto delay_ms = static_cast<int>(arrival(rng));
        std::this_thread::sleep_for(std::chrono::milliseconds(delay_ms));
        customers.emplace_back([&, id = i] {
            if (shop.customer(id)) served.fetch_add(1);
            else balked.fetch_add(1);
        });
    }
    for (auto& c : customers) c.join();

    // Tell the barber to wrap up.
    shop.stop();
    barber_thread.join();

    std::cout << "Served=" << served.load()
              << " Balked=" << balked.load()
              << " Total=" << (served.load() + balked.load())
              << " (expected " << TOTAL_CUSTOMERS << ")\n";
    if (served.load() + balked.load() != TOTAL_CUSTOMERS) {
        std::cerr << "LOST CUSTOMERS — bug!\n";
        return 1;
    }
}
