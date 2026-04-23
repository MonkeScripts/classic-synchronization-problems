# How to Actually Detect Concurrency Bugs

Concurrency bugs hide from casual testing. Here's the toolkit per language.

## General strategy

1. **Inject randomness** — wrap all inter-thread steps with `sleep(random(0..5ms))`. Bugs that hide behind timing become visible.
2. **Run many iterations** — 10,000+ in a loop. Most races manifest ~0.1% of the time.
3. **Use sanitizers/race detectors** — they catch bugs that haven't manifested yet.
4. **Add invariant checks** — e.g. dining philosophers: assert that no two adjacent philosophers are both in EATING state.
5. **Use a timeout** — if your test should finish in 5s but takes 30, you probably deadlocked. Kill it and investigate.

## C++

### Thread Sanitizer (TSan)
```bash
g++ -std=c++20 -fsanitize=thread -g -O1 main.cpp -o main -pthread
./main
```
Catches data races, lock-order-inversion (deadlock potential), misuse of condition variables.

### AddressSanitizer (ASan) + UBSan
```bash
g++ -std=c++20 -fsanitize=address,undefined -g main.cpp -o main -pthread
```
Use/Clang's `-fsanitize=thread` and `-fsanitize=address` are **mutually exclusive** — run separately.

### GDB for deadlocks
```bash
gdb ./main
(gdb) run
# Ctrl-C when it hangs
(gdb) thread apply all bt
```
Look for threads stuck in `pthread_cond_wait` or `pthread_mutex_lock`.

### Helgrind (Valgrind)
```bash
valgrind --tool=helgrind ./main
```
Older, slower than TSan, but catches some things TSan misses.

## Rust

### Built-in: the borrow checker and `Send`/`Sync`
Rust catches many concurrency bugs at compile time. If it compiles, you've avoided whole bug classes. But logic bugs (deadlock, wrong ordering) are still possible.

### Loom
The gold standard for testing concurrent Rust. It explores every possible thread interleaving.
```toml
# Cargo.toml
[target.'cfg(loom)'.dependencies]
loom = "0.7"
```
```rust
#[cfg(loom)]
use loom::sync::Mutex;
#[cfg(not(loom))]
use std::sync::Mutex;

#[test]
#[cfg(loom)]
fn my_test() {
    loom::model(|| {
        // ... your code ...
    });
}
```
Run with: `RUSTFLAGS="--cfg loom" cargo test --test my_test --release`

### ThreadSanitizer via nightly
```bash
RUSTFLAGS="-Z sanitizer=thread" cargo +nightly run
```

### Miri
Detects undefined behavior (including some data races) in unsafe code.
```bash
cargo +nightly miri run
```

## Go

### Race detector — use it always
```bash
go run -race .
go test -race ./...
go build -race
```
**Just always use `-race` in development.** It's the best race detector of the three languages.

### Deadlock detector (built-in!)
Go's runtime detects *full* deadlocks (all goroutines blocked) automatically and panics with a useful message. This is great — but it does NOT detect partial deadlocks where some goroutines are still running.

### goroutine leak detection
```go
import "go.uber.org/goleak"

func TestMain(m *testing.M) {
    goleak.VerifyTestMain(m)
}
```
Tells you if you leaked goroutines — often a sign of a missing channel close or a stuck receiver.

### pprof for stuck goroutines
```go
import _ "net/http/pprof"
// in main: go http.ListenAndServe("localhost:6060", nil)
```
Then `curl localhost:6060/debug/pprof/goroutine?debug=2` shows stack traces of every goroutine. Perfect for diagnosing partial deadlocks.

---

## Invariant check patterns

For each problem, write an assertion that *should* always hold, then check it inside critical sections.

**Dining philosophers:**
```
On transition to EATING:
    assert state[LEFT]  != EATING
    assert state[RIGHT] != EATING
```

**Readers-writers:**
```
global counters: active_readers, active_writers
On read start: active_readers++, assert active_writers == 0
On write start: active_writers++, assert active_readers == 0 and active_writers == 1
```

**Barrier (round R):**
```
Each thread records its current round.
After barrier: all threads are in round R (no thread in R+1 yet).
Implement with: atomic round counter + check on both sides.
```

**Barbershop:**
```
customers counter vs actual waiting threads must always agree.
Use asserts right after lock acquire.
```

These invariants aren't for production — they're for catching YOUR bugs during learning.

---

## A good test loop template

Pseudocode you'll adapt per language:

```
for trial in 1..=10_000:
    setup()
    spawn N threads with random-sleep-injection enabled
    wait for completion with timeout
    if timeout: print "DEADLOCK on trial $trial"; dump state; exit
    check invariants
    check all expected work was done (nothing lost)
```

If this passes 10,000 times with randomized sleeps AND sanitizers clean, your confidence should be high (not infinite — concurrency is humbling).
