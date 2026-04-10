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
target_link_libraries(adapter PRIVATE SystemC::systemc)
