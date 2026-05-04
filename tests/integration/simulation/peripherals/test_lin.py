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
    from tools.testing.virtmcu_test_suite.simulation import Simulation
    from tools.testing.virtmcu_test_suite.transport import SimulationTransport


# LIN protocol helpers
from tools.lin_fbs.virtmcu.lin import LinFrame, LinMessageType

logger = logging.getLogger(__name__)


def create_lin_frame(vtime_ns: int, msg_type: int, data: bytes | None) -> bytearray:
    builder = flatbuffers.Builder(1024)
    data_offset = None
    if data:
        data_offset = builder.CreateByteVector(data)

    LinFrame.Start(builder)
    LinFrame.AddDeliveryVtimeNs(builder, vtime_ns)  # type: ignore
    LinFrame.AddType(builder, msg_type)  # type: ignore
    if data_offset is not None:
        LinFrame.AddData(builder, data_offset)  # type: ignore
    frame = LinFrame.End(builder)
    builder.Finish(frame)
    return bytearray(builder.Output())


@pytest.fixture(scope="module")
def lin_echo_elf(tmp_path_factory: pytest.TempPathFactory) -> Path:
    out = tmp_path_factory.mktemp("lin") / "lin_echo.elf"
    compile_firmware(
        [Path("tests/fixtures/guest_apps/lin_bridge/lin_echo.S")],
        out,
        linker_script=Path("tests/fixtures/guest_apps/lin_bridge/lin_echo.ld"),
    )
    return out


@pytest.fixture
def lin_echo_dtb(tmp_path: Path, sim_transport: SimulationTransport) -> tuple[Path, str]:
    # Use unique topic to avoid interference
    unique_id = hashlib.sha256(tmp_path.name.encode()).hexdigest()[:8]
    lin_topic = SimTopic.lin_unique_prefix(unique_id)

    dtb = tmp_path / "lin_test.dtb"
    compile_dtb(
        Path("tests/fixtures/guest_apps/lin_bridge/lin_test.dts"),
        {"ZENOH_ROUTER_ENDPOINT": sim_transport.dtb_router_endpoint(), '"sim/lin"': f'"{lin_topic}"'},
        dtb,
    )
    return dtb, lin_topic


@pytest.mark.asyncio
async def test_lin_lpuart(
    simulation: Simulation,
    lin_echo_elf: Path,
    lin_echo_dtb: tuple[Path, str],
) -> None:
    dtb, lin_topic = lin_echo_dtb
    kernel = lin_echo_elf

    from tools.testing.virtmcu_test_suite.transport import ZenohTransportImpl
    sim_transport = ZenohTransportImpl(simulation._router, simulation._session)

    # Framework auto-injects clock, router, node_id, and icount.
    # The lpuart device is already declared in lin_test.dts; adding it again
    # via -device would create a duplicate, causing two simultaneous Zenoh
    # connections from the same plugin and breaking the Python mock router.
    extra_args = [
        "-cpu",
        "cortex-a15",
        "-chardev",
        "null,id=n0",
        "-serial",
        "chardev:n0",
        "-net",
        "none",
    ]

    received: list[tuple[int, bytes]] = []


    def on_msg(payload: bytes) -> None:
        try:
            frame = LinFrame.LinFrame.GetRootAsLinFrame(payload, 0)
            msg_type = frame.Type()
            data_len = frame.DataLength()
            data = bytes([frame.Data(i) for i in range(data_len)])  # type: ignore[misc]
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

        from tools.testing.utils import yield_now
        await yield_now()

        frame = create_lin_frame(1_000_000, LinMessageType.LinMessageType.Data, b"X")
        await sim_transport.publish(rx_topic, frame)  # type: ignore[arg-type]
        # Advance clock to process 'X'
        await sim.vta.step(5_000_000)  # LINT_EXCEPTION: vta_step_loop

        logger.info("Sending Break to QEMU RX...")
        frame = create_lin_frame(6_000_000, LinMessageType.LinMessageType.Break, None)
        await sim_transport.publish(rx_topic, frame)  # type: ignore[arg-type]

        # Advance clock to process Break
        await sim.vta.step(5_000_000)  # LINT_EXCEPTION: vta_step_loop

        # Deterministic check for responses
        logger.info("Checking responses...")
        def responses_received() -> bool:
            found_x = False
            found_b = False
            for msg_type, data in received:
                if msg_type == LinMessageType.LinMessageType.Data:
                    if data == b"X":
                        found_x = True
                    if data == b"B":
                        found_b = True
            return found_x and found_b

        try:
            await sim.run_until(responses_received, timeout_ns=50_000_000, step_ns=5_000_000, timeout=10.0)
        except TimeoutError:
            pytest.fail(f"Failed to receive Echo responses. Received: {received}")

        logger.info("SUCCESS: LIN UART verified.")
