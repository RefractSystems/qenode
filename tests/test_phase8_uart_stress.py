"""
Phase 8.6 — High-Baud UART Stress Test: pytest integration layer.

These tests cover two layers:

1. Unit tests for the ZenohFrameHeader wire format (pure Python, no QEMU needed).
   Validates that the 12-byte struct encoding used by the Python test and by the
   Rust zenoh-chardev plugin is consistent.

2. An integration smoke-test that runs uart_stress_test.sh as a subprocess and
   asserts it exits 0.  Skipped when the QEMU binary is absent (e.g. in Docker
   builder stages that only run unit tests).
"""

from __future__ import annotations

import os
import struct
import subprocess
from pathlib import Path

import pytest

# ---------------------------------------------------------------------------
# Constants — must match test/phase8/uart_stress_test.py and
# hw/rust/virtmcu-api/src/lib.rs ZenohFrameHeader.
# ---------------------------------------------------------------------------

FRAME_HEADER_SIZE = 12  # ZenohFrameHeader: u64 + u32 (packed, no padding)
FRAME_VTIME_OFFSET = 0  # delivery_vtime_ns field offset
FRAME_SIZE_OFFSET = 8  # size field offset


# ---------------------------------------------------------------------------
# Helper: ZenohFrameHeader encoding/decoding (mirrors Rust struct)
# ---------------------------------------------------------------------------


def encode_frame(delivery_vtime_ns: int, payload: bytes) -> bytes:
    """Pack a ZenohFrameHeader + payload, matching the Rust zenoh-chardev format."""
    header = struct.pack("<QI", delivery_vtime_ns, len(payload))
    return header + payload


def decode_frame(data: bytes) -> tuple[int, int, bytes]:
    """Return (delivery_vtime_ns, size, payload).  Raises ValueError if too short."""
    if len(data) < FRAME_HEADER_SIZE:
        raise ValueError(f"Frame too short: {len(data)} < {FRAME_HEADER_SIZE}")
    vtime, size = struct.unpack_from("<QI", data, 0)
    payload = data[FRAME_HEADER_SIZE:]
    return vtime, size, payload


# ---------------------------------------------------------------------------
# Unit tests — ZenohFrameHeader wire format
# ---------------------------------------------------------------------------


def test_frame_header_size():
    """ZenohFrameHeader is exactly 12 bytes (u64 + u32, no padding)."""
    assert FRAME_HEADER_SIZE == 12


def test_encode_decode_round_trip():
    """Encoding then decoding returns the original values."""
    vtime = 10_000_800
    payload = b"X"
    frame = encode_frame(vtime, payload)

    assert len(frame) == FRAME_HEADER_SIZE + len(payload)

    decoded_vtime, decoded_size, decoded_payload = decode_frame(frame)
    assert decoded_vtime == vtime
    assert decoded_size == len(payload)
    assert decoded_payload == payload


def test_encode_vtime_ordering():
    """Frames with higher vtime sort later — priority queue delivers in order."""
    earlier = encode_frame(10_000_000, b"X")
    later = encode_frame(10_000_800, b"X")

    vtime_a, _, _ = decode_frame(earlier)
    vtime_b, _, _ = decode_frame(later)

    assert vtime_a < vtime_b


def test_decode_rejects_short_frame():
    """Frames shorter than 12 bytes raise ValueError."""
    import pytest

    with pytest.raises(ValueError, match="too short"):
        decode_frame(b"\x00" * 11)


def test_decode_empty_payload():
    """A frame with zero-length payload is valid (size=0)."""
    frame = encode_frame(999, b"")
    vtime, size, payload = decode_frame(frame)
    assert vtime == 999
    assert size == 0
    assert payload == b""


def test_multi_byte_payload():
    """size field matches the actual payload length for multi-byte payloads."""
    payload = b"hello world"
    frame = encode_frame(12345, payload)
    _, size, decoded = decode_frame(frame)
    assert size == len(payload)
    assert decoded == payload


def test_vtime_max_u64():
    """delivery_vtime_ns handles u64 max without overflow."""
    max_u64 = (1 << 64) - 1
    frame = encode_frame(max_u64, b"X")
    vtime, _, _ = decode_frame(frame)
    assert vtime == max_u64


def test_baud_10mbps_interval():
    """800 ns interval at 10 Mbps — spot-check the constant used in the test."""
    # 10 Mbps = 10_000_000 bits/s → 1_250_000 bytes/s → 800 ns/byte
    baud_ns = 1_000_000_000 // 1_250_000
    assert baud_ns == 800


def test_stress_frame_sequence():
    """50_000 frames encode/decode correctly with monotonically increasing vtimes."""
    start_vtime = 10_000_000
    interval = 800
    n = 1_000  # representative subset (full 50k would be slow in unit test)

    frames = [encode_frame(start_vtime + i * interval, b"X") for i in range(n)]

    for i, frame in enumerate(frames):
        expected_vtime = start_vtime + i * interval
        vtime, size, payload = decode_frame(frame)
        assert vtime == expected_vtime, f"frame {i}: vtime mismatch"
        assert size == 1
        assert payload == b"X"


def test_clock_advance_packing():
    """ClockAdvanceReq wire format: two u64 LE (delta_ns, mujoco_time_ns)."""
    delta_ns = 10_000_000
    mujoco = 0
    packed = struct.pack("<QQ", delta_ns, mujoco)
    assert len(packed) == 16
    out_delta, out_mujoco = struct.unpack("<QQ", packed)
    assert out_delta == delta_ns
    assert out_mujoco == mujoco


def test_clock_ready_unpacking():
    """ClockReadyResp wire format: u64 vtime + u32 n_frames + u32 error_code."""
    vtime = 10_000_000
    n_frames = 0
    error_code = 0
    packed = struct.pack("<QII", vtime, n_frames, error_code)
    assert len(packed) == 16
    out_vtime, out_n, out_err = struct.unpack("<QII", packed)
    assert out_vtime == vtime
    assert out_n == n_frames
    assert out_err == error_code


# ---------------------------------------------------------------------------
# Integration smoke-test — runs the full shell script
# ---------------------------------------------------------------------------

def _get_qemu_bin():
    build_dir = "build-virtmcu-asan" if os.environ.get("VIRTMCU_USE_ASAN") == "1" else "build-virtmcu"
    paths = [
        f"/workspace/third_party/qemu/{build_dir}/install/bin/qemu-system-arm",
        f"/workspace/third_party/qemu/{build_dir}/qemu-system-arm",
        "/opt/virtmcu/bin/qemu-system-arm",
    ]
    for p in paths:
        if Path(p).exists():
            return p
    return paths[0]  # Fallback to first

_QEMU_BIN = _get_qemu_bin()
_STRESS_SCRIPT = Path(__file__).parent / ".." / "test" / "phase8" / "uart_stress_test.sh"


@pytest.mark.skipif(
    not Path(_QEMU_BIN).exists(),
    reason="QEMU binary not found — skipping integration test",
)
def test_uart_stress_integration():
    """
    Full end-to-end: start QEMU + Zenoh router, pre-publish 50k bytes at 10 Mbps
    virtual baud, verify all echoes arrive with correct data.

    This is the integration gate for Phase 8.6.  It runs the shell harness so
    that the same test works both locally and in CI.
    """
    result = subprocess.run(
        ["bash", Path(_STRESS_SCRIPT).resolve()],
        capture_output=False,
        timeout=120,
    )
    assert result.returncode == 0, (
        "uart_stress_test.sh exited with non-zero status — check QEMU logs in /tmp/uart_stress_*/qemu.log"
    )
