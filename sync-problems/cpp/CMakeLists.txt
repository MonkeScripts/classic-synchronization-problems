cmake_minimum_required(VERSION 3.20)
project(sync_problems CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

# Warnings + threading
if(CMAKE_CXX_COMPILER_ID MATCHES "GNU|Clang")
    add_compile_options(-Wall -Wextra -Wpedantic)
endif()

find_package(Threads REQUIRED)

# Enable sanitizers with -DSANITIZER=thread or -DSANITIZER=address
if(SANITIZER)
    message(STATUS "Building with sanitizer: ${SANITIZER}")
    add_compile_options(-fsanitize=${SANITIZER} -g -O1 -fno-omit-frame-pointer)
    add_link_options(-fsanitize=${SANITIZER})
endif()

# Problems — all enabled; comment out ones you haven't started yet
add_subdirectory(dining_philosophers)
add_subdirectory(barrier)
add_subdirectory(barbershop)
add_subdirectory(producer_consumer)
add_subdirectory(readers_writers)
add_subdirectory(h2o)
add_subdirectory(fifo_semaphore)
