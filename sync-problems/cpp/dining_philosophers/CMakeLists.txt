add_executable(dining_philosophers main.cpp)
target_link_libraries(dining_philosophers PRIVATE Threads::Threads)
