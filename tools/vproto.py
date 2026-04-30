# =============================================================================
# vproto.py - Pythonic wrappers for VirtMCU FlatBuffers core protocols.
#
# This module provides @dataclass wrappers around the auto-generated FlatBuffers
# classes. It simplifies packing and unpacking of messages used in the
# mmio-socket-bridge and Zenoh coordination layers.
#
# Prefer using this module over manual struct packing and unpacking.
# =============================================================================

from dataclasses import dataclass

import flatbuffers
from virtmcu.core.ClockAdvanceReq import ClockAdvanceReq as FBClockAdvanceReq
from virtmcu.core.ClockAdvanceReq import CreateClockAdvanceReq
from virtmcu.core.ClockReadyResp import ClockReadyResp as FBClockReadyResp
from virtmcu.core.ClockReadyResp import CreateClockReadyResp
from virtmcu.core.MmioReq import CreateMmioReq
from virtmcu.core.MmioReq import MmioReq as FBMmioReq
from virtmcu.core.SyscMsg import CreateSyscMsg
from virtmcu.core.SyscMsg import SyscMsg as FBSyscMsg
from virtmcu.core.VirtmcuHandshake import CreateVirtmcuHandshake
from virtmcu.core.VirtmcuHandshake import VirtmcuHandshake as FBHandshake
from virtmcu.core.ZenohFrameHeader import CreateZenohFrameHeader
from virtmcu.core.ZenohFrameHeader import ZenohFrameHeader as FBZenohFrameHeader
from virtmcu.core.ZenohSPIHeader import CreateZenohSpiheader
from virtmcu.core.ZenohSPIHeader import ZenohSPIHeader as FBZenohSPIHeader

VIRTMCU_PROTO_MAGIC = 1447904085
VIRTMCU_PROTO_VERSION = 1
MMIO_REQ_READ = 0
MMIO_REQ_WRITE = 1
SYSC_MSG_RESP = 0
SYSC_MSG_IRQ_SET = 1
SYSC_MSG_IRQ_CLEAR = 2

SIZE_VIRTMCU_HANDSHAKE = FBHandshake.SizeOf()
SIZE_MMIO_REQ = FBMmioReq.SizeOf()
SIZE_SYSC_MSG = FBSyscMsg.SizeOf()
SIZE_CLOCK_ADVANCE_REQ = FBClockAdvanceReq.SizeOf()
SIZE_CLOCK_READY_RESP = FBClockReadyResp.SizeOf()
SIZE_ZENOH_FRAME_HEADER = FBZenohFrameHeader.SizeOf()
SIZE_ZENOH_SPI_HEADER = FBZenohSPIHeader.SizeOf()


@dataclass
class VirtmcuHandshake:
    magic: int
    version: int

    @classmethod
    def unpack(cls, data: bytes) -> "VirtmcuHandshake":
        if len(data) < SIZE_VIRTMCU_HANDSHAKE:
            raise ValueError(f"Expected {SIZE_VIRTMCU_HANDSHAKE} bytes")
        fb = FBHandshake()
        fb.Init(data, 0)
        return cls(fb.Magic(), fb.Version())

    def pack(self) -> bytes:
        b = flatbuffers.Builder(32)
        CreateVirtmcuHandshake(b, self.magic, self.version)
        return bytes(b.Bytes[b.Head() :])


@dataclass
class MmioReq:
    type: int
    size: int
    reserved1: int
    reserved2: int
    vtime_ns: int
    addr: int
    data: int

    @classmethod
    def unpack(cls, data: bytes) -> "MmioReq":
        if len(data) < SIZE_MMIO_REQ:
            raise ValueError(f"Expected {SIZE_MMIO_REQ} bytes")
        fb = FBMmioReq()
        fb.Init(data, 0)
        return cls(fb.Type_(), fb.Size(), fb.Reserved1(), fb.Reserved2(), fb.VtimeNs(), fb.Addr(), fb.Data())

    def pack(self) -> bytes:
        b = flatbuffers.Builder(64)
        CreateMmioReq(b, self.type, self.size, self.reserved1, self.reserved2, self.vtime_ns, self.addr, self.data)
        return bytes(b.Bytes[b.Head() :])


@dataclass
class SyscMsg:
    type: int
    irq_num: int
    data: int

    @classmethod
    def unpack(cls, data: bytes) -> "SyscMsg":
        if len(data) < SIZE_SYSC_MSG:
            raise ValueError(f"Expected {SIZE_SYSC_MSG} bytes")
        fb = FBSyscMsg()
        fb.Init(data, 0)
        return cls(fb.Type_(), fb.IrqNum(), fb.Data())

    def pack(self) -> bytes:
        b = flatbuffers.Builder(32)
        CreateSyscMsg(b, self.type, self.irq_num, self.data)
        return bytes(b.Bytes[b.Head() :])


@dataclass
class ClockAdvanceReq:
    delta_ns: int
    mujoco_time_ns: int
    quantum_number: int

    @classmethod
    def unpack(cls, data: bytes) -> "ClockAdvanceReq":
        if len(data) < SIZE_CLOCK_ADVANCE_REQ:
            raise ValueError(f"Expected {SIZE_CLOCK_ADVANCE_REQ} bytes")
        fb = FBClockAdvanceReq()
        fb.Init(data, 0)
        return cls(fb.DeltaNs(), fb.MujocoTimeNs(), fb.QuantumNumber())

    def pack(self) -> bytes:
        b = flatbuffers.Builder(32)
        CreateClockAdvanceReq(b, self.delta_ns, self.mujoco_time_ns, self.quantum_number)
        return bytes(b.Bytes[b.Head() :])


@dataclass
class ClockReadyResp:
    current_vtime_ns: int
    n_frames: int
    error_code: int
    quantum_number: int

    @classmethod
    def unpack(cls, data: bytes) -> "ClockReadyResp":
        if len(data) < SIZE_CLOCK_READY_RESP:
            raise ValueError(f"Expected {SIZE_CLOCK_READY_RESP} bytes")
        fb = FBClockReadyResp()
        fb.Init(data, 0)
        return cls(fb.CurrentVtimeNs(), fb.NFrames(), fb.ErrorCode(), fb.QuantumNumber())

    def pack(self) -> bytes:
        b = flatbuffers.Builder(32)
        CreateClockReadyResp(b, self.current_vtime_ns, self.n_frames, self.error_code, self.quantum_number)
        return bytes(b.Bytes[b.Head() :])


@dataclass
class ZenohFrameHeader:
    delivery_vtime_ns: int
    sequence_number: int
    size: int

    @classmethod
    def unpack(cls, data: bytes) -> "ZenohFrameHeader":
        if len(data) < SIZE_ZENOH_FRAME_HEADER:
            raise ValueError(f"Expected {SIZE_ZENOH_FRAME_HEADER} bytes")
        fb = FBZenohFrameHeader()
        fb.Init(data, 0)
        return cls(fb.DeliveryVtimeNs(), fb.SequenceNumber(), fb.Size())

    def pack(self) -> bytes:
        b = flatbuffers.Builder(32)
        CreateZenohFrameHeader(b, self.delivery_vtime_ns, self.sequence_number, self.size)
        return bytes(b.Bytes[b.Head() :])


@dataclass
class ZenohSPIHeader:
    delivery_vtime_ns: int
    sequence_number: int
    size: int
    cs: bool
    cs_index: int

    @classmethod
    def unpack(cls, data: bytes) -> "ZenohSPIHeader":
        if len(data) < SIZE_ZENOH_SPI_HEADER:
            raise ValueError(f"Expected {SIZE_ZENOH_SPI_HEADER} bytes")
        fb = FBZenohSPIHeader()
        fb.Init(data, 0)
        return cls(fb.DeliveryVtimeNs(), fb.SequenceNumber(), fb.Size(), fb.Cs(), fb.CsIndex())

    def pack(self) -> bytes:
        b = flatbuffers.Builder(32)
        CreateZenohSpiheader(b, self.delivery_vtime_ns, self.sequence_number, self.size, self.cs, self.cs_index, 0)
        return bytes(b.Bytes[b.Head() :])
