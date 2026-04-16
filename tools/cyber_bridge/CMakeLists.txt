cmake_minimum_required(VERSION 3.14...3.31)
project(cyber_bridge CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

find_path(ZENOH_INCLUDE_DIR zenoh.h PATHS /build/zenoh-c/include /opt/virtmcu/include ${CMAKE_SOURCE_DIR}/../../third_party/zenoh-c/include)
find_library(ZENOH_LIBRARY zenohc PATHS /build/zenoh-c/lib /opt/virtmcu/lib ${CMAKE_SOURCE_DIR}/../../third_party/zenoh-c/lib)

if(NOT ZENOH_INCLUDE_DIR OR NOT ZENOH_LIBRARY)
    message(FATAL_ERROR "Zenoh headers or library not found.")
endif()

include_directories(include)
include_directories(${ZENOH_INCLUDE_DIR})

add_executable(resd_replay src/resd_replay.cpp src/resd_parser.cpp)
target_link_libraries(resd_replay PRIVATE ${ZENOH_LIBRARY})

add_executable(mujoco_bridge src/mujoco_bridge.cpp)
target_link_libraries(mujoco_bridge PRIVATE ${ZENOH_LIBRARY})
