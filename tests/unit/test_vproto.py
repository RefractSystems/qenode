"""
SOTA Test Module: test_vproto

Context:
This module implements tests for the test_vproto subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_vproto.
"""

import pytest

from tools.vproto import (
    MMIO_REQ_WRITE,
    SYSC_MSG_IRQ_SET,
    VIRTMCU_PROTO_MAGIC,
    VIRTMCU_PROTO_VERSION,
    ClockAdvanceReq,
    ClockReadyResp,
    MmioReq,
    SyscMsg,
    VirtmcuHandshake,
    ZenohFrameHeader,
    ZenohSPIHeader,
)


def test_virtmcu_handshake_pack_unpack():
    hs = VirtmcuHandshake(magic=VIRTMCU_PROTO_MAGIC, version=VIRTMCU_PROTO_VERSION)
    packed = hs.pack()
    assert len(packed) == 8

    unpacked = VirtmcuHandshake.unpack(packed)
    assert unpacked.magic == VIRTMCU_PROTO_MAGIC
    assert unpacked.version == VIRTMCU_PROTO_VERSION


def test_virtmcu_handshake_unpack_error():
    with pytest.raises(ValueError, match="Expected 8 bytes"):
        VirtmcuHandshake.unpack(b"1234")


def test_mmio_req_pack_unpack():
    req = MmioReq(
        type=MMIO_REQ_WRITE,
        size=4,
        reserved1=0,
        reserved2=0,
        vtime_ns=1234567890,
        addr=0x40001000,
        data=0xDEADBEEF,
    )
    packed = req.pack()
    # 1+1+2+4 + 8 + 8 + 8 = 32
    assert len(packed) == 32

    unpacked = MmioReq.unpack(packed)
    assert unpacked.type == MMIO_REQ_WRITE
    assert unpacked.size == 4
    assert unpacked.reserved1 == 0
    assert unpacked.reserved2 == 0
    assert unpacked.vtime_ns == 1234567890
    assert unpacked.addr == 0x40001000
    assert unpacked.data == 0xDEADBEEF


def test_mmio_req_unpack_error():
    with pytest.raises(ValueError, match="Expected 32 bytes"):
        MmioReq.unpack(b"1234567890")


def test_sysc_msg_pack_unpack():
    msg = SyscMsg(
        type=SYSC_MSG_IRQ_SET,
        irq_num=5,
        data=0,
    )
    packed = msg.pack()
    # 4+4+8 = 16
    assert len(packed) == 16

    unpacked = SyscMsg.unpack(packed)
    assert unpacked.type == SYSC_MSG_IRQ_SET
    assert unpacked.irq_num == 5
    assert unpacked.data == 0


def test_sysc_msg_unpack_error():
    with pytest.raises(ValueError, match="Expected 16 bytes"):
        SyscMsg.unpack(b"short")


def test_clock_advance_req_pack_unpack():
    req = ClockAdvanceReq(delta_ns=1000000, mujoco_time_ns=2000000, quantum_number=123)
    packed = req.pack()
    assert len(packed) == 24

    unpacked = ClockAdvanceReq.unpack(packed)
    assert unpacked.delta_ns == 1000000
    assert unpacked.mujoco_time_ns == 2000000
    assert unpacked.quantum_number == 123


def test_clock_advance_req_unpack_error():
    with pytest.raises(ValueError, match="Expected 24 bytes"):
        ClockAdvanceReq.unpack(b"toolittle")


def test_clock_ready_resp_pack_unpack():
    resp = ClockReadyResp(current_vtime_ns=5000000, n_frames=10, error_code=0, quantum_number=123)
    packed = resp.pack()
    assert len(packed) == 24

    unpacked = ClockReadyResp.unpack(packed)
    assert unpacked.current_vtime_ns == 5000000
    assert unpacked.n_frames == 10
    assert unpacked.error_code == 0
    assert unpacked.quantum_number == 123


def test_clock_ready_resp_unpack_error():
    with pytest.raises(ValueError, match="Expected 24 bytes"):
        ClockReadyResp.unpack(b"wrongsize")


def test_zenoh_frame_header_pack_unpack():
    hdr = ZenohFrameHeader(delivery_vtime_ns=1000, sequence_number=42, size=1024)
    packed = hdr.pack()
    assert len(packed) == 24
    unpacked = ZenohFrameHeader.unpack(packed)
    assert unpacked.delivery_vtime_ns == 1000
    assert unpacked.sequence_number == 42
    assert unpacked.size == 1024


def test_spi_header_pack_unpack():
    hdr = ZenohSPIHeader(
        delivery_vtime_ns=5000,
        sequence_number=17,
        size=4,
        cs=True,
        cs_index=1,
    )
    packed = hdr.pack()
    assert len(packed) == 24
    unpacked = ZenohSPIHeader.unpack(packed)
    assert unpacked.delivery_vtime_ns == 5000
    assert unpacked.sequence_number == 17
    assert unpacked.size == 4
    assert unpacked.cs is True
    assert unpacked.cs_index == 1
