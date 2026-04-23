add_executable(producer_consumer main.cpp)
target_link_libraries(producer_consumer PRIVATE Threads::Threads)
