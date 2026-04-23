add_executable(barrier main.cpp)
target_link_libraries(barrier PRIVATE Threads::Threads)
