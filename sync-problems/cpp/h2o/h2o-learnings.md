# H2O (C++): Mistakes & Learnings

A short record of implementing the water-factory problem in C++20 with the lecture's WaterFactory3 strategy.

---

## What we did

Mirrored slide-set WaterFactory3: two counting semaphores cap the type counts (`hydrogen_sem_{2}`, `oxygen_sem_{1}`) and a `std::barrier<>{3}` gates the start of bonding so all 3 atoms enter `bond()` together.

```cpp
void hydrogen(int id) {
    hydrogen_sem_.acquire();   // at most 2 H past this line
    barrier.arrive_and_wait(); // wait until 2 H + 1 O are all here
    bond_h(id);
    hydrogen_sem_.release();   // let the next H in
}

void oxygen(int id) {
    oxygen_sem_.acquire();     // at most 1 O past this line
    barrier.arrive_and_wait();
    bond_o(id);
    oxygen_sem_.release();
}
```

Why both primitives are needed:
- Semaphores alone (WaterFactory2) → atoms bond solo without a partner.
- Barrier alone (WaterFactory1) → 3 oxygens can bond into ozone.
- Together → semaphores enforce the **type count**, barrier enforces the **start signal**.

10 clean runs: `H bonds: 100/100  O bonds: 50/50  max simultaneous in bond: 3` every time.

---

## Mistakes

### `std::barrier<> barrier;` — no default constructor

```cpp
class WaterFactory {
private:
    std::barrier<> barrier;   // ❌ won't compile
};
```

```
error: no matching function for call to 'std::barrier<>::barrier()'
note: candidate: 'std::barrier(ptrdiff_t __count, _CompletionF __completion = ...)'
note:   candidate expects 2 arguments, 0 provided
```

`std::barrier` requires the expected count at construction — there is no default. Same shape as `std::counting_semaphore<>{N}` right next to it.

### Fix
```cpp
std::barrier<> barrier{3};   // 2 H + 1 O = 3 atoms per molecule
```

### The takeaway
> If the surrounding fields use brace-init with a count (`semaphore<>{2}`, `semaphore<>{1}`), the barrier should too. Default-construction works for `std::mutex`, `std::condition_variable`, `std::atomic<T>` — but anything that *carries a sized state* (barrier, latch, semaphore) needs the count up front.

---

## Build

```bash
cd sync-problems/cpp
cmake -S . -B build && cmake --build build
./build/h2o/h2o
```
