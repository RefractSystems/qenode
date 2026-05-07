from __future__ import annotations

# =============================================================================
# vproto.py - Pythonic wrappers for VirtMCU FlatBuffers core protocols.
#
# This module provides @dataclass wrappers around the auto-generated FlatBuffers
# classes. It simplifies packing and unpacking of messages used in the
# mmio-socket-bridge and Zenoh coordination layers.
#
# Prefer using this module over manual struct packing and unpacking.
# =============================================================================
import logging
from dataclasses import dataclass

import flatbuffers

from generated.virtmcu.core.ClockAdvanceReq import ClockAdvanceReq as FBClockAdvanceReq
from generated.virtmcu.core.ClockAdvanceReq import CreateClockAdvanceReq
from generated.virtmcu.core.ClockReadyResp import ClockReadyResp as FBClockReadyResp
from generated.virtmcu.core.ClockReadyResp import CreateClockReadyResp
from generated.virtmcu.core.CoordDoneReq import CoordDoneReq as FBCoordDoneReq
from generated.virtmcu.core.CoordDoneReq import (
    CoordDoneReqAddMessages,
    CoordDoneReqAddQuantum,
    CoordDoneReqAddVtimeLimit,
    CoordDoneReqEnd,
    CoordDoneReqStart,
    CoordDoneReqStartMessagesVector,
)
from generated.virtmcu.core.CoordMessage import CoordMessage as FBCoordMessage
from generated.virtmcu.core.CoordMessage import (
    CoordMessageAddDeliveryVtimeNs,
    CoordMessageAddDstNodeId,
    CoordMessageAddPayload,
    CoordMessageAddProtocol,
    CoordMessageAddSequenceNumber,
    CoordMessageAddSrcNodeId,
    CoordMessageEnd,
    CoordMessageStart,
)
from generated.virtmcu.core.MmioReq import CreateMmioReq
from generated.virtmcu.core.MmioReq import MmioReq as FBMmioReq
from generated.virtmcu.core.SyscMsg import CreateSyscMsg
from generated.virtmcu.core.SyscMsg import SyscMsg as FBSyscMsg
from generated.virtmcu.core.VirtmcuHandshake import CreateVirtmcuHandshake
from generated.virtmcu.core.VirtmcuHandshake import VirtmcuHandshake as FBHandshake
from generated.virtmcu.core.ZenohFrameHeader import CreateZenohFrameHeader
from generated.virtmcu.core.ZenohFrameHeader import ZenohFrameHeader as FBZenohFrameHeader
from generated.virtmcu.core.ZenohSPIHeader import CreateZenohSpiheader
from generated.virtmcu.core.ZenohSPIHeader import ZenohSPIHeader as FBZenohSPIHeader
from generated.virtmcu.rf802154.Rf802154Header import Rf802154Header as FBRf802154Header
from generated.virtmcu.rf802154.Rf802154Header import (
    Rf802154HeaderAddDeliveryVtimeNs,
    Rf802154HeaderAddLqi,
    Rf802154HeaderAddRssi,
    Rf802154HeaderAddSequenceNumber,
    Rf802154HeaderAddSize,
    Rf802154HeaderEnd,
    Rf802154HeaderStart,
)

logger = logging.getLogger(__name__)

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
    def unpack(cls, data: bytes) -> VirtmcuHandshake:
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
    def unpack(cls, data: bytes) -> MmioReq:
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
    def unpack(cls, data: bytes) -> SyscMsg:
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
    absolute_vtime_ns: int
    quantum_number: int

    @classmethod
    def unpack(cls, data: bytes) -> ClockAdvanceReq:
        if len(data) < SIZE_CLOCK_ADVANCE_REQ:
            raise ValueError(f"Expected {SIZE_CLOCK_ADVANCE_REQ} bytes")
        fb = FBClockAdvanceReq()
        fb.Init(data, 0)
        return cls(fb.DeltaNs(), fb.AbsoluteVtimeNs(), fb.QuantumNumber())

    def pack(self) -> bytes:
        b = flatbuffers.Builder(32)
        CreateClockAdvanceReq(b, self.delta_ns, self.absolute_vtime_ns, self.quantum_number)
        return bytes(b.Bytes[b.Head() :])


@dataclass
class ClockReadyResp:
    current_vtime_ns: int
    n_frames: int
    error_code: int
    quantum_number: int

    @classmethod
    def unpack(cls, data: bytes) -> ClockReadyResp:
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
    def unpack(cls, data: bytes) -> ZenohFrameHeader:
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
    def unpack(cls, data: bytes) -> ZenohSPIHeader:
        if len(data) < SIZE_ZENOH_SPI_HEADER:
            raise ValueError(f"Expected {SIZE_ZENOH_SPI_HEADER} bytes")
        fb = FBZenohSPIHeader()
        fb.Init(data, 0)
        return cls(fb.DeliveryVtimeNs(), fb.SequenceNumber(), fb.Size(), fb.Cs(), fb.CsIndex())

    def pack(self) -> bytes:
        b = flatbuffers.Builder(32)
        CreateZenohSpiheader(b, self.delivery_vtime_ns, self.sequence_number, self.size, self.cs, self.cs_index, 0)
        return bytes(b.Bytes[b.Head() :])


@dataclass
class Rf802154Header:
    delivery_vtime_ns: int
    sequence_number: int
    size: int
    rssi: int
    lqi: int

    @classmethod
    def unpack(cls, data: bytes) -> Rf802154Header:
        # Tables in VirtMCU are size-prefixed (4 bytes)
        if len(data) < 4:
            raise ValueError("Data too short for size prefix")
        fb = FBRf802154Header.GetRootAs(data, 4)
        return cls(
            fb.DeliveryVtimeNs(),
            fb.SequenceNumber(),
            fb.Size(),
            fb.Rssi(),
            fb.Lqi(),
        )

    def pack(self) -> bytes:
        b = flatbuffers.Builder(64)
        Rf802154HeaderStart(b)
        Rf802154HeaderAddDeliveryVtimeNs(b, self.delivery_vtime_ns)
        Rf802154HeaderAddSequenceNumber(b, self.sequence_number)
        Rf802154HeaderAddSize(b, self.size)
        Rf802154HeaderAddRssi(b, self.rssi)
        Rf802154HeaderAddLqi(b, self.lqi)
        res = Rf802154HeaderEnd(b)
        b.FinishSizePrefixed(res)
        return bytes(b.Output())


@dataclass
class CoordMessage:
    src_node_id: int
    dst_node_id: int
    delivery_vtime_ns: int
    sequence_number: int
    protocol: int
    payload: bytes

    @classmethod
    def unpack(cls, data: bytes) -> CoordMessage:
        fb = FBCoordMessage.GetRootAs(data, 0)
        payload = bytes(fb.Payload(i) for i in range(fb.PayloadLength()))  # type: ignore[misc]
        return cls(
            fb.SrcNodeId(),
            fb.DstNodeId(),
            fb.DeliveryVtimeNs(),
            fb.SequenceNumber(),
            fb.Protocol(),
            payload,
        )

    def _pack_to_builder(self, b: flatbuffers.Builder) -> int:
        payload_offset = b.CreateByteVector(self.payload)
        CoordMessageStart(b)
        CoordMessageAddSrcNodeId(b, self.src_node_id)
        CoordMessageAddDstNodeId(b, self.dst_node_id)
        CoordMessageAddDeliveryVtimeNs(b, self.delivery_vtime_ns)
        CoordMessageAddSequenceNumber(b, self.sequence_number)
        CoordMessageAddProtocol(b, self.protocol)
        CoordMessageAddPayload(b, payload_offset)
        return int(CoordMessageEnd(b))

    def pack(self) -> bytes:
        b = flatbuffers.Builder(1024)
        res = self._pack_to_builder(b)
        b.Finish(res)
        return bytes(b.Output())


@dataclass
class CoordDoneReq:
    quantum: int
    vtime_limit: int
    messages: list[CoordMessage]

    @classmethod
    def unpack(cls, data: bytes) -> CoordDoneReq:
        fb = FBCoordDoneReq.GetRootAs(data, 0)
        msgs = []
        for i in range(fb.MessagesLength()):
            m_fb = fb.Messages(i)
            if m_fb is not None:
                payload = bytes(m_fb.Payload(j) for j in range(m_fb.PayloadLength()))
                msgs.append(
                    CoordMessage(
                        m_fb.SrcNodeId(),
                        m_fb.DstNodeId(),
                        m_fb.DeliveryVtimeNs(),
                        m_fb.SequenceNumber(),
                        m_fb.Protocol(),
                        payload,
                    )
                )
        return cls(fb.Quantum(), fb.VtimeLimit(), msgs)

    def pack(self) -> bytes:
        b = flatbuffers.Builder(1024)
        msg_offsets: list[int] = []
        for m in reversed(self.messages):
            msg_offsets.insert(0, m._pack_to_builder(b))

        CoordDoneReqStartMessagesVector(b, len(msg_offsets))
        for offset in reversed(msg_offsets):
            b.PrependUOffsetTRelative(offset)
        messages_vector = b.EndVector()

        CoordDoneReqStart(b)
        CoordDoneReqAddQuantum(b, self.quantum)
        CoordDoneReqAddVtimeLimit(b, self.vtime_limit)
        CoordDoneReqAddMessages(b, messages_vector)
        res = CoordDoneReqEnd(b)
        b.Finish(res)
        return bytes(b.Output())
