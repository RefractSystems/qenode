"""
SOTA Test Module: test_lin

Context:
This module implements tests for the test_lin subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_lin.
"""

from __future__ import annotations

import hashlib
import logging
from pathlib import Path
from typing import TYPE_CHECKING

import flatbuffers
import pytest

from tools.testing.virtmcu_test_suite.factory import compile_dtb, compile_firmware
from tools.testing.virtmcu_test_suite.topics import SimTopic

if TYPE_CHECKING:
    import zenoh

    from tools.testing.virtmcu_test_suite.simulation import Simulation
    from tools.testing.virtmcu_test_suite.transport import SimulationTransport


# LIN protocol helpers
from virtmcu.lin import LinFrame, LinMessageType

logger = logging.getLogger(__name__)


def create_lin_frame(vtime_ns: int, msg_type: int, data: bytes | None) -> bytearray:
    builder = flatbuffers.Builder(1024)
    data_offset = None
    if data:
        data_offset = builder.CreateByteVector(data)

    LinFrame.Start(builder)
    LinFrame.AddDeliveryVtimeNs(builder, vtime_ns)
    LinFrame.AddType(builder, msg_type)
    if data_offset is not None:
        LinFrame.AddData(builder, data_offset)
    frame = LinFrame.End(builder)
    builder.Finish(frame)
    return bytearray(builder.Output())


@pytest.mark.asyncio
async def test_lin_lpuart(
    simulation: Simulation,
    sim_transport: SimulationTransport,
    zenoh_session: zenoh.Session,
    zenoh_router: str,
    tmp_path: Path,
) -> None:

    tmpdir = tmp_path

    # 1. Build ELF
    kernel = Path(tmpdir) / "lin_echo.elf"
    compile_firmware(
        [Path("tests/fixtures/guest_apps/lin_bridge/lin_echo.S")],
        kernel,
        linker_script=Path("tests/fixtures/guest_apps/lin_bridge/lin_echo.ld"),
    )

    # Use unique topic to avoid interference
    unique_id = hashlib.sha256(tmp_path.name.encode()).hexdigest()[:8]
    lin_topic = SimTopic.lin_unique_prefix(unique_id)

    # Generate DTB
    dtb = Path(tmpdir) / "lin_test.dtb"
    compile_dtb(
        Path("tests/fixtures/guest_apps/lin_bridge/lin_test.dts"),
        {"ZENOH_ROUTER_ENDPOINT": sim_transport.dtb_router_endpoint(), '"sim/lin"': f'"{lin_topic}"'},
        dtb,
    )
    extra_args = [
        "-cpu",
        "cortex-a15",
        "-chardev",
        "null,id=n0",
        "-serial",
        "chardev:n0",
        "-icount",
        "shift=0,align=off,sleep=off",
        "-net",
        "none",
        "-device",
        sim_transport.get_clock_device_str(node_id=0),
        "-device",
        f"s32k144-lpuart,node=0,{sim_transport.get_peripheral_props()},topic={lin_topic}",
    ]

    received: list[tuple[int, bytes]] = []

    def on_msg(payload: bytes) -> None:
        try:
            frame = LinFrame.LinFrame.GetRootAsLinFrame(payload, 0)
            msg_type = frame.Type()
            data_len = frame.DataLength()
            data = bytes([frame.Data(i) for i in range(data_len)])
            logger.info(f"Received from QEMU: type={msg_type}, data={data!r}")
            received.append((msg_type, data))
        except Exception as e:  # noqa: BLE001
            # Subscription callbacks should not crash the transport thread on malformed payloads
            logger.error(f"Callback error: {e}")

    tx_topic = f"{lin_topic}/0/tx"
    rx_topic = f"{lin_topic}/0/rx"

    await sim_transport.subscribe(tx_topic, on_msg)

    simulation.transport = sim_transport
    simulation.add_node(node_id=0, dtb=dtb, kernel=kernel, extra_args=extra_args)

    async with simulation as sim:
        assert sim.vta is not None
        logger.info(f"QEMU started with topic {lin_topic}; sending 'X' to RX...")

        frame = create_lin_frame(1_000_000, LinMessageType.LinMessageType.Data, b"X")
        await sim_transport.publish(rx_topic, frame)  # type: ignore[arg-type]

        # Advance clock to process 'X'
        await sim.vta.step(5_000_000)

        logger.info("Sending Break to QEMU RX...")
        frame = create_lin_frame(6_000_000, LinMessageType.LinMessageType.Break, None)
        await sim_transport.publish(rx_topic, frame)  # type: ignore[arg-type]

        # Advance clock to process Break
        await sim.vta.step(5_000_000)

        # Deterministic check for responses
        logger.info("Checking responses...")
        found_x = False
        found_b = False
        for _ in range(10):
            for msg_type, data in received:
                if msg_type == LinMessageType.LinMessageType.Data:
                    if data == b"X":
                        found_x = True
                    if data == b"B":
                        found_b = True
            if found_x and found_b:
                break
            await sim.vta.step(5_000_000)

        assert found_x, f"Failed to receive Echo for 'X', received: {received}"
        assert found_b, f"Failed to receive Echo for Break, received: {received}"

        logger.info("SUCCESS: LIN UART verified.")
