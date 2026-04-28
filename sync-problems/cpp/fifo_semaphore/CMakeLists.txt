add_executable(fifo_semaphore main.cpp)
target_link_libraries(fifo_semaphore PRIVATE Threads::Threads)

add_executable(fifo_semaphore_queue queue.cpp)
target_link_libraries(fifo_semaphore_queue PRIVATE Threads::Threads)
