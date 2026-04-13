cmake_minimum_required(VERSION 3.14)
project(systemc_adapter CXX C)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

include(FetchContent)

# Fetch SystemC
FetchContent_Declare(
  systemc
  GIT_REPOSITORY https://github.com/accellera-official/systemc.git
  GIT_TAG        2.3.3
)

# Fetch Xilinx libsystemctlm-soc for Remote Port
FetchContent_Declare(
  systemctlm_soc
  GIT_REPOSITORY https://github.com/Xilinx/libsystemctlm-soc.git
  GIT_TAG        master
  GIT_SUBMODULES ""
)

# SystemC CMake uses C++14 by default, we force it to match ours
set(CMAKE_CXX_STANDARD 17 CACHE STRING "C++ standard to build all targets." FORCE)
set(ENABLE_PHASE_CALLBACKS ON CACHE BOOL "" FORCE)

FetchContent_MakeAvailable(systemc systemctlm_soc)

add_executable(adapter main.cpp)
add_executable(rp_adapter remote_port_adapter.cpp)

find_path(ZENOH_INCLUDE_DIR zenoh.h PATHS /build/zenoh-c/include /opt/virtmcu/include ${CMAKE_SOURCE_DIR}/../../third_party/zenoh-c/include)
find_library(ZENOH_LIBRARY zenohc PATHS /build/zenoh-c/lib /opt/virtmcu/lib ${CMAKE_SOURCE_DIR}/../../third_party/zenoh-c/lib)

target_include_directories(adapter PRIVATE ${ZENOH_INCLUDE_DIR})
target_link_libraries(adapter PRIVATE SystemC::systemc ${ZENOH_LIBRARY})

target_include_directories(rp_adapter PRIVATE ${ZENOH_INCLUDE_DIR} ${systemctlm_soc_SOURCE_DIR} ${systemctlm_soc_SOURCE_DIR}/libremote-port)
target_link_libraries(rp_adapter PRIVATE SystemC::systemc ${ZENOH_LIBRARY})
# libsystemctlm-soc provides C and C++ source files that need to be compiled.
# Let's add them to rp_adapter.
target_sources(rp_adapter PRIVATE
  ${systemctlm_soc_SOURCE_DIR}/libremote-port/remote-port-sk.c
  ${systemctlm_soc_SOURCE_DIR}/libremote-port/remote-port-proto.c
  ${systemctlm_soc_SOURCE_DIR}/libremote-port/remote-port-tlm.cc
  ${systemctlm_soc_SOURCE_DIR}/libremote-port/remote-port-tlm-memory-master.cc
  ${systemctlm_soc_SOURCE_DIR}/libremote-port/remote-port-tlm-memory-slave.cc
  ${systemctlm_soc_SOURCE_DIR}/libremote-port/remote-port-tlm-wires.cc
  ${systemctlm_soc_SOURCE_DIR}/libremote-port/remote-port-tlm-ats.cc
  ${systemctlm_soc_SOURCE_DIR}/libremote-port/safeio.c
)
