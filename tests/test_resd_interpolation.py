import os
import struct
import subprocess

import pytest

# Paths
WORKSPACE_DIR = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
BUILD_DIR = os.path.join(WORKSPACE_DIR, "tools/cyber_bridge/build")
TEST_INTERPOL_BIN = os.path.join(BUILD_DIR, "test_resd_interpolation")


@pytest.fixture(scope="module", autouse=True)
def build_test_interpol():
    os.makedirs(BUILD_DIR, exist_ok=True)
    cpp_source = """
#include <iostream>
#include <iomanip>
#include "virtmcu/resd_parser.hpp"
using namespace virtmcu;
int main(int argc, char* argv[]) {
    if (argc < 3) return 1;
    ResdParser parser(argv[1]);
    if (!parser.init()) return 1;

    uint64_t vtime = std::stoull(argv[2]);
    auto sensor = parser.get_sensor(ResdSampleType::ACCELERATION, 0);
    auto vals = sensor->get_reading(vtime);

    std::cout << std::fixed << std::setprecision(6);
    for (size_t i = 0; i < vals.size(); ++i) {
        std::cout << vals[i] << (i == vals.size() - 1 ? "" : ",");
    }
    std::cout << std::endl;
    return 0;
}
"""
    with open(os.path.join(BUILD_DIR, "test_resd_interpolation.cpp"), "w") as f:
        f.write(cpp_source)

    cmd = [
        "g++",
        "-std=c++17",
        "-I" + os.path.join(WORKSPACE_DIR, "tools/cyber_bridge/include"),
        "-I" + os.path.join(WORKSPACE_DIR, "third_party/zenoh-c/include"),
        os.path.join(BUILD_DIR, "test_resd_interpolation.cpp"),
        os.path.join(WORKSPACE_DIR, "tools/cyber_bridge/src/resd_parser.cpp"),
        "-o",
        TEST_INTERPOL_BIN,
    ]
    subprocess.run(cmd, check=True)


def get_reading(resd_file, vtime_ns):
    res = subprocess.run([TEST_INTERPOL_BIN, str(resd_file), str(vtime_ns)], capture_output=True, text=True)
    if res.returncode != 0:
        return None
    return [float(x) for x in res.stdout.strip().split(",")]


def test_linear_interpolation(tmp_path):
    resd = tmp_path / "interpol.resd"
    with open(resd, "wb") as f:
        f.write(b"RESD")
        f.write(struct.pack("<B", 1))
        f.write(b"\x00\x00\x00")

        # Block: ACCELERATION
        f.write(struct.pack("<BHH", 0x01, 0x0002, 0))
        f.write(struct.pack("<Q", 8 + 8 + 2 * 20))  # data_size: start_time(8) + metadata_size(8) + 2 samples
        f.write(struct.pack("<Q", 0))  # start_time
        f.write(struct.pack("<Q", 0))  # metadata_size

        # Sample 1: t=1000, x=100, y=200, z=300
        f.write(struct.pack("<Qiii", 1000, 100, 200, 300))
        # Sample 2: t=2000, x=200, y=400, z=600
        f.write(struct.pack("<Qiii", 2000, 200, 400, 600))

    # Midpoint t=1500 -> should be x=150, y=300, z=450
    vals = get_reading(resd, 1500)
    assert vals == [150.0, 300.0, 450.0]

    # 25% t=1250 -> should be x=125, y=250, z=375
    vals = get_reading(resd, 1250)
    assert vals == [125.0, 250.0, 375.0]

    # Before first t=500 -> zero-order hold (or clamping to first sample)
    # Current implementation: if vtime < samples[0].ts, returns samples[0]
    vals = get_reading(resd, 500)
    assert vals == [100.0, 200.0, 300.0]

    # After last t=2500 -> zero-order hold (returns last sample)
    vals = get_reading(resd, 2500)
    assert vals == [200.0, 400.0, 600.0]
