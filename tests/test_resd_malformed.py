import os
import struct
import subprocess

import pytest

# Paths
WORKSPACE_DIR = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
BUILD_DIR = os.path.join(WORKSPACE_DIR, "tools/cyber_bridge/build")
TEST_PARSER_BIN = os.path.join(BUILD_DIR, "test_parser_internal")


@pytest.fixture(scope="module", autouse=True)
def build_test_parser():
    """Builds a dedicated test parser binary if it doesn't exist."""
    os.makedirs(BUILD_DIR, exist_ok=True)

    # Inline C++ test driver for internal parsing checks
    cpp_source = """
#include <iostream>
#include "virtmcu/resd_parser.hpp"
using namespace virtmcu;
int main(int argc, char* argv[]) {
    if (argc < 2) return 1;
    ResdParser parser(argv[1]);
    if (!parser.init()) {
        std::cerr << "INIT_FAILED" << std::endl;
        return 1;
    }
    std::cout << "INIT_SUCCESS" << std::endl;
    return 0;
}
"""
    with open(os.path.join(BUILD_DIR, "test_parser_internal.cpp"), "w") as f:
        f.write(cpp_source)

    # Compile
    cmd = [
        "g++",
        "-std=c++17",
        "-I" + os.path.join(WORKSPACE_DIR, "tools/cyber_bridge/include"),
        "-I" + os.path.join(WORKSPACE_DIR, "third_party/zenoh-c/include"),
        os.path.join(BUILD_DIR, "test_parser_internal.cpp"),
        os.path.join(WORKSPACE_DIR, "tools/cyber_bridge/src/resd_parser.cpp"),
        "-o",
        TEST_PARSER_BIN,
    ]
    subprocess.run(cmd, check=True)


def run_parser(filename):
    res = subprocess.run([TEST_PARSER_BIN, filename], capture_output=True, text=True)
    return res.returncode, res.stdout, res.stderr


def test_resd_invalid_magic(tmp_path):
    resd = tmp_path / "invalid_magic.resd"
    with open(resd, "wb") as f:
        f.write(b"NOTR")  # Wrong magic
        f.write(struct.pack("<B", 1))
        f.write(b"\x00\x00\x00")

    code, out, err = run_parser(str(resd))
    assert code != 0
    assert "Invalid magic" in err


def test_resd_truncated_header(tmp_path):
    resd = tmp_path / "truncated_header.resd"
    with open(resd, "wb") as f:
        f.write(b"RESD")
        f.write(struct.pack("<B", 1))
        # Missing 3 bytes padding

    code, out, err = run_parser(str(resd))
    assert code != 0
    # The current parser doesn't check file.read() success for padding, but it might fail on next read


def test_resd_unknown_block_type(tmp_path):
    resd = tmp_path / "unknown_block.resd"
    with open(resd, "wb") as f:
        f.write(b"RESD")
        f.write(struct.pack("<B", 1))
        f.write(b"\x00\x00\x00")

        # Block: unknown type 0xFF
        f.write(struct.pack("<BHH", 0xFF, 0x0002, 0))  # block_type, sample_type, channel
        f.write(struct.pack("<Q", 10))  # data_size
        f.write(b"A" * 10)  # dummy data

    code, out, err = run_parser(str(resd))
    # Current implementation skips unknown blocks
    assert code == 0
    assert "INIT_SUCCESS" in out


def test_resd_truncated_data(tmp_path):
    resd = tmp_path / "truncated_data.resd"
    with open(resd, "wb") as f:
        f.write(b"RESD")
        f.write(struct.pack("<B", 1))
        f.write(b"\x00\x00\x00")

        # Block header says 100 bytes, but we only give 10
        f.write(struct.pack("<BHH", 0x01, 0x0002, 0))
        f.write(struct.pack("<Q", 100))
        f.write(struct.pack("<Q", 0))  # start_time
        f.write(struct.pack("<Q", 0))  # metadata_size
        f.write(b"A" * 10)  # truncated data

    code, out, err = run_parser(str(resd))
    # This should fail if it tries to read beyond EOF
    # The current implementation might loop or read garbage
    assert code != 0


def test_resd_unsupported_sample_type(tmp_path):
    resd = tmp_path / "unsupported_sample.resd"
    with open(resd, "wb") as f:
        f.write(b"RESD")
        f.write(struct.pack("<B", 1))
        f.write(b"\x00\x00\x00")

        # Block: unsupported type 0x9999
        f.write(struct.pack("<BHH", 0x01, 0x9999, 0))
        f.write(struct.pack("<Q", 16))  # data_size: start_time(8) + metadata_size(8)
        f.write(struct.pack("<Q", 0))  # start_time
        f.write(struct.pack("<Q", 0))  # metadata_size

    code, out, err = run_parser(str(resd))
    # Should skip unsupported samples but not fail
    assert code == 0
    assert "INIT_SUCCESS" in out
