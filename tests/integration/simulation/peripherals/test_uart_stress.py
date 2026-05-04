"""
SOTA Test Module: test_uart_stress

Objective:
Verify that the UART peripheral can handle high-throughput bursts
without dropping data or losing synchronization, using an external
Zenoh-based stress testing tool.
"""

from __future__ import annotations

import asyncio
import os
from typing import TYPE_CHECKING, Any

import pytest

from tools import vproto

if TYPE_CHECKING:
    from tools.testing.virtmcu_test_suite.simulation import Simulation


# ---------------------------------------------------------------------------
# Unit tests — ZenohFrameHeader wire format
# ---------------------------------------------------------------------------

FRAME_HEADER_SIZE = vproto.SIZE_ZENOH_FRAME_HEADER


def encode_frame(delivery_vtime_ns: int, payload: bytes, sequence: int = 0) -> bytes:
    header = vproto.ZenohFrameHeader(delivery_vtime_ns, sequence, len(payload)).pack()
    return header + payload


def decode_frame(data: bytes) -> tuple[int, int, int, bytes]:
    if len(data) < FRAME_HEADER_SIZE:
        raise ValueError(f"Frame too short: {len(data)} < {FRAME_HEADER_SIZE}")
    header = vproto.ZenohFrameHeader.unpack(data[:FRAME_HEADER_SIZE])
    payload = data[FRAME_HEADER_SIZE:]
    return header.delivery_vtime_ns, header.sequence_number, header.size, payload


def test_frame_header_size() -> None:
    assert FRAME_HEADER_SIZE == 24


def test_encode_decode_round_trip() -> None:
    vtime = 10_000_800
    seq = 42
    payload = b"X"
    frame = encode_frame(vtime, payload, seq)
    decoded_vtime, decoded_seq, decoded_size, decoded_payload = decode_frame(frame)
    assert decoded_vtime == vtime
    assert decoded_seq == seq
    assert decoded_size == len(payload)
    assert decoded_payload == payload


def test_encode_vtime_ordering() -> None:
    earlier = encode_frame(10_000_000, b"X")
    later = encode_frame(10_000_800, b"X")
    vtime_a, _, _, _ = decode_frame(earlier)
    vtime_b, _, _, _ = decode_frame(later)
    assert vtime_a < vtime_b


def test_decode_rejects_short_frame() -> None:
    with pytest.raises(ValueError, match="too short"):
        decode_frame(b"\x00" * (FRAME_HEADER_SIZE - 1))


def test_decode_empty_payload() -> None:
    frame = encode_frame(999, b"")
    vtime, seq, size, payload = decode_frame(frame)
    assert vtime == 999
    assert seq == 0
    assert size == 0
    assert payload == b""


def test_multi_byte_payload() -> None:
    payload = b"hello world"
    frame = encode_frame(12345, payload)
    _, _, size, decoded = decode_frame(frame)
    assert size == len(payload)
    assert decoded == payload


def test_vtime_max_u64() -> None:
    max_u64 = (1 << 64) - 1
    frame = encode_frame(max_u64, b"X")
    vtime, _, _, _ = decode_frame(frame)
    assert vtime == max_u64


def test_baud_10mbps_interval() -> None:
    baud_ns = 1_000_000_000 // 1_250_000
    assert baud_ns == 800


def test_stress_frame_sequence() -> None:
    start_vtime = 10_000_000
    interval = 800
    n = 1_000
    frames = [encode_frame(start_vtime + i * interval, b"X") for i in range(n)]
    for i, frame in enumerate(frames):
        vtime, _, _, _ = decode_frame(frame)
        assert vtime == start_vtime + i * interval


def test_clock_advance_packing() -> None:
    delta_ns = 10_000_000
    packed = vproto.ClockAdvanceReq(delta_ns, 0, 0).pack()
    req = vproto.ClockAdvanceReq.unpack(packed)
    assert req.delta_ns == delta_ns


def test_clock_ready_unpacking() -> None:
    vtime = 10_000_000
    packed = vproto.ClockReadyResp(vtime, 0, 0, 0).pack()
    resp = vproto.ClockReadyResp.unpack(packed)
    assert resp.current_vtime_ns == vtime


# ---------------------------------------------------------------------------
# Integration Test — Orchestration Inversion
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
@pytest.mark.timeout(900)
async def test_uart_stress_integration(simulation: Simulation, guest_app_factory: Any) -> None:  # noqa: ANN401
    """
    Modernized integration test: Uses the simulation fixture to launch QEMU
    and entirely orchestrates the stress test via the SimulationTransport.
    """
    app_dir = guest_app_factory("uart_echo")
    kernel = app_dir / "echo.elf"

    app_dir_boot = guest_app_factory("boot_arm")
    dtb = app_dir_boot / "minimal.dtb"

    # Use a dynamic timeout and scale down work for ASan
    is_asan = os.environ.get("VIRTMCU_USE_ASAN") == "1"
    total_bytes = 10_000 if is_asan else 50_000
    chunk_size = 1024
    baud_1mbps_interval_ns = 10_000
    start_vtime_ns = 10_000_000
    test_byte = b"X"
    test_byte_val = ord("X")

    # Use a realistic buffer size that matches hardware rather than a hack
    extra_args = [
        "-icount",
        "shift=6,align=off,sleep=off",
        "-chardev",
        f"virtmcu,id=uart0,node=0,max-backlog=1000000,router={simulation._router},topic=virtmcu/uart",
        "-serial",
        "chardev:uart0",
        "-device",
        f"virtmcu-clock,node=0,mode=slaved-icount,router={simulation._router}",
    ]

    simulation.add_node(node_id=0, dtb=dtb, kernel=kernel, extra_args=extra_args)

    received_bytes = bytearray()
    received_string = ""
    welcome_detected = asyncio.Event()
    received_all = asyncio.Event()

    # 1. Subscribe to UART TX (what the firmware sends back to us)
    def on_tx_sample(payload: bytes) -> None:
        nonlocal received_string
        if len(payload) < vproto.SIZE_ZENOH_FRAME_HEADER:
            return
        data = payload[vproto.SIZE_ZENOH_FRAME_HEADER :]
        if not data:
            return

        if not welcome_detected.is_set():
            received_string += data.decode("utf-8", errors="replace")
            if "Interactive UART Echo Ready" in received_string:
                welcome_detected.set()

        # We only care about echoed 'X's for the throughput validation
        for b in data:
            if b == test_byte_val:
                received_bytes.append(b)
                if len(received_bytes) >= total_bytes:
                    received_all.set()

    if simulation.transport is None:
        from tools.testing.virtmcu_test_suite.transport import ZenohTransportImpl

        simulation.transport = ZenohTransportImpl(simulation._router, simulation._session)

    from tools.testing.virtmcu_test_suite.topics import SimTopic

    async with simulation as sim:
        assert sim.transport is not None
        # 1. Setup subscription for echoed data
        await sim.transport.subscribe(SimTopic.uart_tx(0), on_tx_sample)

        # 2. Wait for the firmware to boot and print the welcome message by pumping the clock
        try:
            await sim.run_until(
                welcome_detected.is_set,
                timeout_ns=500_000_000,
                step_ns=10_000_000,
                timeout=15.0,
            )
        except TimeoutError:
            pytest.fail("Firmware did not print welcome message")

        # 3. Blast data in chunks, and wait for each chunk to be echoed before sending the next.
        for i in range(0, total_bytes, chunk_size):
            chunk_end = min(i + chunk_size, total_bytes)
            for j in range(i, chunk_end):
                vtime = start_vtime_ns + (j * baud_1mbps_interval_ns)
                header = vproto.ZenohFrameHeader(vtime, 0, 1).pack()
                await sim.transport.publish(SimTopic.uart_rx(0), header + test_byte)

            # Wait for this chunk to be echoed back by advancing the clock
            def chunk_received(target: int = chunk_end) -> bool:
                return len(received_bytes) >= target

            try:
                # Give it plenty of virtual time per chunk, and step in larger increments to speed up wall-clock
                await sim.run_until(chunk_received, timeout_ns=500_000_000, step_ns=10_000_000, timeout=30.0)
            except TimeoutError:
                pytest.fail(
                    f"Stress test timed out on chunk {i}-{chunk_end}. Received {len(received_bytes)} / {chunk_end} bytes"
                )
        assert len(received_bytes) == total_bytes
