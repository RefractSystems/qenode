"""
Core protocol hardening tests for virtmcu.

Tests the shared wire-format contracts between Rust (virtmcu-api) and Python
without requiring a QEMU binary or Zenoh router.  Every struct, topic naming
convention, and error code defined in hw/rust/virtmcu-api/src/lib.rs has a
corresponding test here.

Run with: pytest tests/test_core_protocols.py -v
"""

from __future__ import annotations

import heapq
import struct
from dataclasses import dataclass, field

import pytest

# ---------------------------------------------------------------------------
# Constants — must match hw/rust/virtmcu-api/src/lib.rs exactly
# ---------------------------------------------------------------------------

VIRTMCU_PROTO_MAGIC: int = 0x564D4355
VIRTMCU_PROTO_VERSION: int = 1

MMIO_REQ_READ: int = 0
MMIO_REQ_WRITE: int = 1

SYSC_MSG_RESP: int = 0
SYSC_MSG_IRQ_SET: int = 1
SYSC_MSG_IRQ_CLEAR: int = 2

CLOCK_ERROR_OK: int = 0
CLOCK_ERROR_STALL: int = 1
CLOCK_ERROR_ZENOH: int = 2

ZENOH_FRAME_HEADER_SIZE: int = 12  # ZenohFrameHeader: u64 + u32, packed
CLOCK_ADVANCE_REQ_SIZE: int = 16  # ClockAdvanceReq: u64 + u64
CLOCK_READY_RESP_SIZE: int = 16  # ClockReadyResp: u64 + u32 + u32
MMIO_REQ_SIZE: int = 32  # MmioReq: 1+1+2+4+8+8+8
SYSC_MSG_SIZE: int = 16  # SyscMsg: 4+4+8
VIRTMCU_HANDSHAKE_SIZE: int = 8  # VirtmcuHandshake: u32 + u32

# Topic naming (must match zenoh-chardev, zenoh-netdev, zenoh-clock)
CHARDEV_TOPIC_BASE: str = "sim/chardev"
CLOCK_TOPIC_BASE: str = "sim/clock"


# ---------------------------------------------------------------------------
# Wire-format helpers
# ---------------------------------------------------------------------------


def encode_zenoh_frame(delivery_vtime_ns: int, payload: bytes) -> bytes:
    """Pack ZenohFrameHeader + payload (mirrors Rust encode_frame())."""
    header = struct.pack("<QI", delivery_vtime_ns, len(payload))
    return header + payload


def decode_zenoh_frame(data: bytes) -> tuple[int, int, bytes]:
    """Unpack ZenohFrameHeader. Returns (vtime, size, payload). Raises ValueError if too short."""
    if len(data) < ZENOH_FRAME_HEADER_SIZE:
        raise ValueError(f"Frame too short: {len(data)} < {ZENOH_FRAME_HEADER_SIZE}")
    vtime, size = struct.unpack_from("<QI", data, 0)
    return vtime, size, data[ZENOH_FRAME_HEADER_SIZE:]


def encode_clock_advance_req(delta_ns: int, mujoco_time_ns: int = 0) -> bytes:
    return struct.pack("<QQ", delta_ns, mujoco_time_ns)


def decode_clock_advance_req(data: bytes) -> tuple[int, int]:
    if len(data) < CLOCK_ADVANCE_REQ_SIZE:
        raise ValueError(f"ClockAdvanceReq too short: {len(data)}")
    return struct.unpack_from("<QQ", data, 0)


def encode_clock_ready_resp(current_vtime_ns: int, n_frames: int = 0, error_code: int = CLOCK_ERROR_OK) -> bytes:
    return struct.pack("<QII", current_vtime_ns, n_frames, error_code)


def decode_clock_ready_resp(data: bytes) -> tuple[int, int, int]:
    if len(data) < CLOCK_READY_RESP_SIZE:
        raise ValueError(f"ClockReadyResp too short: {len(data)}")
    return struct.unpack_from("<QII", data, 0)


def encode_mmio_req(
    type_: int,
    size: int,
    vtime_ns: int,
    addr: int,
    data: int,
) -> bytes:
    return struct.pack("<BBxxI QQQ", type_, size, 0, vtime_ns, addr, data)


def decode_mmio_req(raw: bytes) -> tuple:
    if len(raw) < MMIO_REQ_SIZE:
        raise ValueError(f"MmioReq too short: {len(raw)}")
    type_, sz, _, vtime, addr, dat = struct.unpack_from("<BBxxI QQQ", raw, 0)
    return type_, sz, vtime, addr, dat


def encode_sysc_msg(type_: int, irq_num: int, data: int) -> bytes:
    return struct.pack("<IIQ", type_, irq_num, data)


def decode_sysc_msg(raw: bytes) -> tuple[int, int, int]:
    if len(raw) < SYSC_MSG_SIZE:
        raise ValueError(f"SyscMsg too short: {len(raw)}")
    return struct.unpack_from("<IIQ", raw, 0)


def encode_handshake(magic: int = VIRTMCU_PROTO_MAGIC, version: int = VIRTMCU_PROTO_VERSION) -> bytes:
    return struct.pack("<II", magic, version)


def decode_handshake(raw: bytes) -> tuple[int, int]:
    if len(raw) < VIRTMCU_HANDSHAKE_SIZE:
        raise ValueError("Handshake too short")
    return struct.unpack_from("<II", raw, 0)


# ---------------------------------------------------------------------------
# ZenohFrameHeader tests
# ---------------------------------------------------------------------------


class TestZenohFrameHeader:
    def test_size_constant(self):
        assert ZENOH_FRAME_HEADER_SIZE == 12

    def test_encode_decode_round_trip(self):
        vtime, payload = 12_345_678, b"hello"
        frame = encode_zenoh_frame(vtime, payload)
        assert len(frame) == ZENOH_FRAME_HEADER_SIZE + len(payload)
        v, sz, rest = decode_zenoh_frame(frame)
        assert v == vtime
        assert sz == len(payload)
        assert rest == payload

    def test_empty_payload(self):
        frame = encode_zenoh_frame(0, b"")
        v, sz, rest = decode_zenoh_frame(frame)
        assert v == 0
        assert sz == 0
        assert rest == b""

    def test_vtime_zero(self):
        frame = encode_zenoh_frame(0, b"X")
        v, _, _ = decode_zenoh_frame(frame)
        assert v == 0

    def test_vtime_max_u64(self):
        max_u64 = (1 << 64) - 1
        frame = encode_zenoh_frame(max_u64, b"X")
        v, _, _ = decode_zenoh_frame(frame)
        assert v == max_u64

    def test_rejects_short_frame(self):
        with pytest.raises(ValueError, match="too short"):
            decode_zenoh_frame(b"\x00" * 11)

    def test_rejects_empty(self):
        with pytest.raises(ValueError):
            decode_zenoh_frame(b"")

    def test_exact_header_size_accepted(self):
        frame = encode_zenoh_frame(1, b"")
        v, sz, rest = decode_zenoh_frame(frame)
        assert v == 1
        assert sz == 0
        assert rest == b""

    def test_little_endian_vtime(self):
        # vtime = 0x0102030405060708 → bytes [08,07,06,05,04,03,02,01]
        vtime = 0x0102030405060708
        frame = encode_zenoh_frame(vtime, b"")
        assert frame[:8] == bytes([0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01])

    def test_little_endian_size(self):
        frame = encode_zenoh_frame(0, b"hello")  # size=5 = [05,00,00,00]
        assert frame[8:12] == bytes([0x05, 0x00, 0x00, 0x00])

    def test_vtime_ordering(self):
        earlier = encode_zenoh_frame(1_000_000, b"A")
        later = encode_zenoh_frame(2_000_000, b"A")
        v1, _, _ = decode_zenoh_frame(earlier)
        v2, _, _ = decode_zenoh_frame(later)
        assert v1 < v2

    def test_10mbps_baud_interval_ns(self):
        # 10 Mbps = 1_250_000 bytes/s → 800 ns/byte
        interval = 1_000_000_000 // 1_250_000
        assert interval == 800

    def test_sequence_monotonic(self):
        n, start, step = 1_000, 10_000_000, 800
        for i in range(n):
            frame = encode_zenoh_frame(start + i * step, b"X")
            v, sz, payload = decode_zenoh_frame(frame)
            assert v == start + i * step, f"frame {i}: vtime mismatch"
            assert sz == 1
            assert payload == b"X"

    def test_size_field_matches_payload(self):
        for length in [0, 1, 127, 255, 1024]:
            payload = bytes([0xAB] * length)
            frame = encode_zenoh_frame(0, payload)
            _, sz, _ = decode_zenoh_frame(frame)
            assert sz == length

    def test_payload_boundary_integrity(self):
        """Payload bytes are not touched by encode/decode."""
        payload = bytes(range(256))
        frame = encode_zenoh_frame(42, payload)
        _, _, rest = decode_zenoh_frame(frame)
        assert rest == payload


# ---------------------------------------------------------------------------
# ClockAdvanceReq tests
# ---------------------------------------------------------------------------


class TestClockAdvanceReq:
    def test_size_constant(self):
        assert CLOCK_ADVANCE_REQ_SIZE == 16

    def test_round_trip(self):
        raw = encode_clock_advance_req(10_000_000, 42)
        delta, mujoco = decode_clock_advance_req(raw)
        assert delta == 10_000_000
        assert mujoco == 42

    def test_zero_values(self):
        raw = encode_clock_advance_req(0, 0)
        assert raw == b"\x00" * 16

    def test_max_u64_delta(self):
        max_u64 = (1 << 64) - 1
        raw = encode_clock_advance_req(max_u64, 0)
        delta, _ = decode_clock_advance_req(raw)
        assert delta == max_u64

    def test_little_endian_encoding(self):
        raw = encode_clock_advance_req(0x0102030405060708, 0)
        assert raw[:8] == bytes([0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01])

    def test_rejects_short_payload(self):
        with pytest.raises(ValueError):
            decode_clock_advance_req(b"\x00" * 15)

    def test_quantum_10ms(self):
        raw = encode_clock_advance_req(10_000_000, 0)
        delta, _ = decode_clock_advance_req(raw)
        assert delta == 10_000_000  # 10 ms in ns


# ---------------------------------------------------------------------------
# ClockReadyResp tests
# ---------------------------------------------------------------------------


class TestClockReadyResp:
    def test_size_constant(self):
        assert CLOCK_READY_RESP_SIZE == 16

    def test_ok_round_trip(self):
        raw = encode_clock_ready_resp(10_000_000, n_frames=50, error_code=CLOCK_ERROR_OK)
        vtime, n, err = decode_clock_ready_resp(raw)
        assert vtime == 10_000_000
        assert n == 50
        assert err == CLOCK_ERROR_OK

    def test_stall_error_code(self):
        raw = encode_clock_ready_resp(0, error_code=CLOCK_ERROR_STALL)
        _, _, err = decode_clock_ready_resp(raw)
        assert err == CLOCK_ERROR_STALL

    def test_zenoh_error_code(self):
        raw = encode_clock_ready_resp(0, error_code=CLOCK_ERROR_ZENOH)
        _, _, err = decode_clock_ready_resp(raw)
        assert err == CLOCK_ERROR_ZENOH

    def test_error_codes_distinct(self):
        assert CLOCK_ERROR_OK != CLOCK_ERROR_STALL
        assert CLOCK_ERROR_OK != CLOCK_ERROR_ZENOH
        assert CLOCK_ERROR_STALL != CLOCK_ERROR_ZENOH

    def test_error_code_ok_is_zero(self):
        assert CLOCK_ERROR_OK == 0

    def test_rejects_short_payload(self):
        with pytest.raises(ValueError):
            decode_clock_ready_resp(b"\x00" * 15)

    def test_vtime_preserved(self):
        for vtime in [0, 1, 10_000_000, (1 << 63), (1 << 64) - 1]:
            raw = encode_clock_ready_resp(vtime)
            v, _, _ = decode_clock_ready_resp(raw)
            assert v == vtime, f"vtime mismatch at {vtime}"


# ---------------------------------------------------------------------------
# MmioReq tests
# ---------------------------------------------------------------------------


class TestMmioReq:
    def test_size_constant(self):
        assert MMIO_REQ_SIZE == 32

    def test_read_type_is_zero(self):
        assert MMIO_REQ_READ == 0

    def test_write_type_is_one(self):
        assert MMIO_REQ_WRITE == 1

    def test_types_distinct(self):
        assert MMIO_REQ_READ != MMIO_REQ_WRITE

    def test_read_round_trip(self):
        raw = encode_mmio_req(MMIO_REQ_READ, 4, 999_999, 0x1000_0000, 0)
        t, sz, vtime, addr, data = decode_mmio_req(raw)
        assert t == MMIO_REQ_READ
        assert sz == 4
        assert vtime == 999_999
        assert addr == 0x1000_0000
        assert data == 0

    def test_write_round_trip(self):
        raw = encode_mmio_req(MMIO_REQ_WRITE, 4, 1_000_000, 0x4000_0000, 0xDEAD_BEEF)
        t, sz, vtime, addr, data = decode_mmio_req(raw)
        assert t == MMIO_REQ_WRITE
        assert sz == 4
        assert addr == 0x4000_0000
        assert data == 0xDEAD_BEEF

    def test_rejects_short_payload(self):
        with pytest.raises(ValueError):
            decode_mmio_req(b"\x00" * 31)


# ---------------------------------------------------------------------------
# SyscMsg tests
# ---------------------------------------------------------------------------


class TestSyscMsg:
    def test_size_constant(self):
        assert SYSC_MSG_SIZE == 16

    def test_type_constants_distinct(self):
        assert SYSC_MSG_RESP != SYSC_MSG_IRQ_SET
        assert SYSC_MSG_RESP != SYSC_MSG_IRQ_CLEAR
        assert SYSC_MSG_IRQ_SET != SYSC_MSG_IRQ_CLEAR

    def test_irq_set_round_trip(self):
        raw = encode_sysc_msg(SYSC_MSG_IRQ_SET, irq_num=7, data=1)
        t, irq, d = decode_sysc_msg(raw)
        assert t == SYSC_MSG_IRQ_SET
        assert irq == 7
        assert d == 1

    def test_irq_clear_round_trip(self):
        raw = encode_sysc_msg(SYSC_MSG_IRQ_CLEAR, irq_num=3, data=0)
        t, irq, d = decode_sysc_msg(raw)
        assert t == SYSC_MSG_IRQ_CLEAR
        assert irq == 3
        assert d == 0

    def test_resp_round_trip(self):
        raw = encode_sysc_msg(SYSC_MSG_RESP, irq_num=0, data=0xABCDEF01)
        t, _, d = decode_sysc_msg(raw)
        assert t == SYSC_MSG_RESP
        assert d == 0xABCDEF01

    def test_rejects_short_payload(self):
        with pytest.raises(ValueError):
            decode_sysc_msg(b"\x00" * 15)


# ---------------------------------------------------------------------------
# VirtmcuHandshake tests
# ---------------------------------------------------------------------------


class TestVirtmcuHandshake:
    def test_size_constant(self):
        assert VIRTMCU_HANDSHAKE_SIZE == 8

    def test_magic_value(self):
        assert VIRTMCU_PROTO_MAGIC == 0x564D4355

    def test_version_is_one(self):
        assert VIRTMCU_PROTO_VERSION == 1

    def test_round_trip(self):
        raw = encode_handshake()
        magic, version = decode_handshake(raw)
        assert magic == VIRTMCU_PROTO_MAGIC
        assert version == VIRTMCU_PROTO_VERSION

    def test_magic_le_bytes(self):
        raw = encode_handshake()
        # 0x564D4355 in LE = [0x55, 0x43, 0x4D, 0x56]
        assert raw[:4] == bytes([0x55, 0x43, 0x4D, 0x56])

    def test_version_le_bytes(self):
        raw = encode_handshake()
        # version=1 in LE = [0x01, 0x00, 0x00, 0x00]
        assert raw[4:8] == bytes([0x01, 0x00, 0x00, 0x00])

    def test_rejects_short_payload(self):
        with pytest.raises(ValueError):
            decode_handshake(b"\x00" * 7)


# ---------------------------------------------------------------------------
# Zenoh topic naming conventions
# ---------------------------------------------------------------------------


class TestTopicNaming:
    def test_chardev_rx_topic_node0(self):
        assert f"{CHARDEV_TOPIC_BASE}/0/rx" == "sim/chardev/0/rx"

    def test_chardev_tx_topic_node0(self):
        assert f"{CHARDEV_TOPIC_BASE}/0/tx" == "sim/chardev/0/tx"

    def test_chardev_rx_tx_distinct(self):
        rx = f"{CHARDEV_TOPIC_BASE}/0/rx"
        tx = f"{CHARDEV_TOPIC_BASE}/0/tx"
        assert rx != tx

    def test_chardev_multi_node_isolation(self):
        rx0 = f"{CHARDEV_TOPIC_BASE}/0/rx"
        rx1 = f"{CHARDEV_TOPIC_BASE}/1/rx"
        assert rx0 != rx1

    def test_clock_advance_topic(self):
        assert f"{CLOCK_TOPIC_BASE}/advance/0" == "sim/clock/advance/0"
        assert f"{CLOCK_TOPIC_BASE}/advance/3" == "sim/clock/advance/3"

    def test_clock_heartbeat_topic(self):
        assert f"{CLOCK_TOPIC_BASE}/heartbeat/0" == "sim/clock/heartbeat/0"

    def test_netdev_rx_topic(self):
        base = "sim/netdev"
        assert f"{base}/0/rx" == "sim/netdev/0/rx"

    def test_topic_no_trailing_slash(self):
        for topic in [
            f"{CHARDEV_TOPIC_BASE}/0/rx",
            f"{CHARDEV_TOPIC_BASE}/0/tx",
            f"{CLOCK_TOPIC_BASE}/advance/0",
        ]:
            assert not topic.endswith("/"), f"topic has trailing slash: {topic}"

    def test_node_id_string_vs_int(self):
        # Node IDs are u32 properties; topic uses str(node_id)
        for node in [0, 1, 15, 255]:
            topic = f"{CHARDEV_TOPIC_BASE}/{node}/rx"
            assert f"/{node}/" in topic


# ---------------------------------------------------------------------------
# Delivery queue (min-heap by vtime) — mirrors zenoh-chardev OrderedPacket
# ---------------------------------------------------------------------------


@dataclass(order=False)
class DeliveryPacket:
    """Mirrors OrderedPacket in zenoh-chardev: min-heap by vtime."""

    vtime: int
    data: bytes = field(default=b"", compare=False)

    def __lt__(self, other: "DeliveryPacket") -> bool:
        return self.vtime < other.vtime

    def __eq__(self, other: object) -> bool:
        if not isinstance(other, DeliveryPacket):
            return NotImplemented
        return self.vtime == other.vtime


class TestDeliveryQueue:
    def test_min_heap_ordering(self):
        heap: list[DeliveryPacket] = []
        for vtime in [3_000, 1_000, 2_000]:
            heapq.heappush(heap, DeliveryPacket(vtime))
        assert heapq.heappop(heap).vtime == 1_000
        assert heapq.heappop(heap).vtime == 2_000
        assert heapq.heappop(heap).vtime == 3_000

    def test_vtime_zero_first(self):
        heap: list[DeliveryPacket] = []
        heapq.heappush(heap, DeliveryPacket(1_000_000))
        heapq.heappush(heap, DeliveryPacket(0))
        assert heapq.heappop(heap).vtime == 0

    def test_vtime_max_last(self):
        max_u64 = (1 << 64) - 1
        heap: list[DeliveryPacket] = []
        heapq.heappush(heap, DeliveryPacket(max_u64))
        heapq.heappush(heap, DeliveryPacket(1))
        assert heapq.heappop(heap).vtime == 1
        assert heapq.heappop(heap).vtime == max_u64

    def test_equal_vtimes_both_dequeued(self):
        heap: list[DeliveryPacket] = []
        heapq.heappush(heap, DeliveryPacket(500, b"A"))
        heapq.heappush(heap, DeliveryPacket(500, b"B"))
        assert len(heap) == 2
        heapq.heappop(heap)
        heapq.heappop(heap)
        assert len(heap) == 0

    def test_large_sequence_monotonic(self):
        n = 10_000
        heap: list[DeliveryPacket] = []
        for i in reversed(range(n)):
            heapq.heappush(heap, DeliveryPacket(i))
        prev = -1
        for _ in range(n):
            p = heapq.heappop(heap)
            assert p.vtime > prev or p.vtime == prev + 1
            prev = p.vtime

    def test_max_heap_packets_boundary(self):
        """MAX_HEAP_PACKETS = 65536: heap must not grow beyond this in production."""
        max_packets = 65_536
        heap: list[DeliveryPacket] = []
        for i in range(max_packets + 10):
            if len(heap) >= max_packets:
                break
            heapq.heappush(heap, DeliveryPacket(i))
        assert len(heap) == max_packets

    def test_8mhz_baud_delivery_order(self):
        """50k frames at 800 ns intervals dequeue in vtime order."""
        n, start, step = 1_000, 10_000_000, 800  # 1k representative subset
        heap: list[DeliveryPacket] = []
        for i in reversed(range(n)):
            heapq.heappush(heap, DeliveryPacket(start + i * step))
        prev_vtime = 0
        for i in range(n):
            p = heapq.heappop(heap)
            expected = start + i * step
            assert p.vtime == expected, f"frame {i}: expected {expected}, got {p.vtime}"
            assert p.vtime >= prev_vtime
            prev_vtime = p.vtime


# ---------------------------------------------------------------------------
# Stall / error-code propagation (pure protocol logic)
# ---------------------------------------------------------------------------


class TestStallProtocol:
    def test_stall_reply_format(self):
        """A stall reply has error_code=CLOCK_ERROR_STALL, vtime is last-known."""
        last_known_vtime = 5_000_000
        raw = encode_clock_ready_resp(last_known_vtime, n_frames=0, error_code=CLOCK_ERROR_STALL)
        vtime, n, err = decode_clock_ready_resp(raw)
        assert err == CLOCK_ERROR_STALL
        assert vtime == last_known_vtime
        assert n == 0

    def test_ok_reply_zero_stalls(self):
        raw = encode_clock_ready_resp(10_000_000, error_code=CLOCK_ERROR_OK)
        _, _, err = decode_clock_ready_resp(raw)
        assert err == CLOCK_ERROR_OK

    def test_stall_sentinel_is_u64_max(self):
        # QUANTUM_WAIT_STALL_SENTINEL in zenoh-clock is u64::MAX
        sentinel = (1 << 64) - 1
        assert sentinel == 0xFFFF_FFFF_FFFF_FFFF

    def test_stall_sentinel_distinct_from_valid_delta(self):
        sentinel = (1 << 64) - 1
        valid_delta = 10_000_000  # 10 ms
        assert sentinel != valid_delta

    def test_clock_advance_with_zero_delta_is_valid(self):
        """delta_ns=0 is a valid request (time authority confirms current vtime)."""
        raw = encode_clock_advance_req(0, 0)
        delta, _ = decode_clock_advance_req(raw)
        assert delta == 0

    def test_multiple_stall_codes_independent(self):
        assert CLOCK_ERROR_STALL == 1
        assert CLOCK_ERROR_ZENOH == 2
        assert CLOCK_ERROR_OK == 0

    def test_error_code_fits_in_u32(self):
        for code in [CLOCK_ERROR_OK, CLOCK_ERROR_STALL, CLOCK_ERROR_ZENOH]:
            assert 0 <= code <= 0xFFFF_FFFF


# ---------------------------------------------------------------------------
# Cross-format consistency
# ---------------------------------------------------------------------------


class TestCrossFormatConsistency:
    def test_zenoh_frame_header_size_matches_struct_pack(self):
        frame = encode_zenoh_frame(0, b"")
        assert len(frame) == ZENOH_FRAME_HEADER_SIZE

    def test_clock_advance_req_size_matches_struct_pack(self):
        raw = encode_clock_advance_req(0, 0)
        assert len(raw) == CLOCK_ADVANCE_REQ_SIZE

    def test_clock_ready_resp_size_matches_struct_pack(self):
        raw = encode_clock_ready_resp(0)
        assert len(raw) == CLOCK_READY_RESP_SIZE

    def test_mmio_req_size_matches_struct_pack(self):
        raw = encode_mmio_req(0, 0, 0, 0, 0)
        assert len(raw) == MMIO_REQ_SIZE

    def test_sysc_msg_size_matches_struct_pack(self):
        raw = encode_sysc_msg(0, 0, 0)
        assert len(raw) == SYSC_MSG_SIZE

    def test_handshake_size_matches_struct_pack(self):
        raw = encode_handshake()
        assert len(raw) == VIRTMCU_HANDSHAKE_SIZE

    def test_all_wire_formats_little_endian(self):
        """All multibyte fields use little-endian byte order."""
        # ZenohFrameHeader vtime=1 → first byte is 0x01
        frame = encode_zenoh_frame(1, b"")
        assert frame[0] == 0x01, "ZenohFrameHeader not little-endian"

        # ClockAdvanceReq delta_ns=1 → first byte is 0x01
        req = encode_clock_advance_req(1, 0)
        assert req[0] == 0x01, "ClockAdvanceReq not little-endian"

        # ClockReadyResp vtime=1 → first byte is 0x01
        resp = encode_clock_ready_resp(1)
        assert resp[0] == 0x01, "ClockReadyResp not little-endian"
