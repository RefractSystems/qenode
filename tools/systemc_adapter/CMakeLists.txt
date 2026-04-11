cmake_minimum_required(VERSION 3.14)
project(systemc_adapter CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

include(FetchContent)

# Fetch SystemC
FetchContent_Declare(
  systemc
  GIT_REPOSITORY https://github.com/accellera-official/systemc.git
  GIT_TAG        3.0.0
)

# SystemC CMake uses C++14 by default, we force it to match ours
set(CMAKE_CXX_STANDARD 17 CACHE STRING "C++ standard to build all targets." FORCE)
set(ENABLE_PHASE_CALLBACKS ON CACHE BOOL "" FORCE)

FetchContent_MakeAvailable(systemc)

add_executable(adapter main.cpp)
find_path(ZENOH_INCLUDE_DIR zenoh.h PATHS /build/zenoh-c/include /opt/virtmcu/include ${CMAKE_SOURCE_DIR}/../../third_party/zenoh-c/include)
find_library(ZENOH_LIBRARY zenohc PATHS /build/zenoh-c/lib /opt/virtmcu/lib ${CMAKE_SOURCE_DIR}/../../third_party/zenoh-c/lib)

target_include_directories(adapter PRIVATE ${ZENOH_INCLUDE_DIR})
target_link_libraries(adapter PRIVATE SystemC::systemc ${ZENOH_LIBRARY})
