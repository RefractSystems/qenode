"""
SOTA Test Module: test_lin

Context:
This module implements tests for the test_lin subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_lin.
"""

import logging
import sys
from pathlib import Path

import flatbuffers
import pytest

from tools.testing.virtmcu_test_suite.factory import compile_dtb, compile_firmware

# Add tools/lin_fbs to sys.path
sys.path.append(str(Path.cwd() / "tools/lin_fbs"))

from virtmcu.lin import LinFrame, LinMessageType

logger = logging.getLogger(__name__)


def create_lin_frame(vtime_ns, msg_type, data):
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
    return builder.Output()


@pytest.mark.asyncio
async def test_lin_lpuart(qemu_launcher, sim_transport, zenoh_session, zenoh_router):
    import shutil
    import tempfile

    tmpdir = tempfile.mkdtemp(prefix="virtmcu-lin-")

    # 1. Build ELF
    kernel = Path(tmpdir) / "lin_echo.elf"
    compile_firmware(
        [Path("tests/fixtures/guest_apps/lin_bridge/lin_echo.S")],
        kernel,
        linker_script=Path("tests/fixtures/guest_apps/lin_bridge/lin_echo.ld"),
    )

    # Use unique topic to avoid interference
    import uuid

    unique_id = str(uuid.uuid4())[:8]
    lin_topic = f"sim/lin/{unique_id}"

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

    received = []

    def on_msg(payload):
        try:
            frame = LinFrame.LinFrame.GetRootAsLinFrame(payload, 0)
            msg_type = frame.Type()
            data_len = frame.DataLength()
            data = bytes([frame.Data(i) for i in range(data_len)])
            logger.info(f"Received from QEMU: type={msg_type}, data={data!r}")
            received.append((msg_type, data))
        except Exception as e:
            logger.error(f"Callback error: {e}")

    tx_topic = f"{lin_topic}/0/tx"
    rx_topic = f"{lin_topic}/0/rx"

    await sim_transport.subscribe(tx_topic, on_msg)

    logger.info(f"Starting QEMU with topic {lin_topic} via Orchestrator...")
    from tools.testing.virtmcu_test_suite.orchestrator import SimulationOrchestrator

    orchestrator = SimulationOrchestrator(zenoh_session, zenoh_router, qemu_launcher)
    orchestrator.transport = sim_transport

    try:
        orchestrator.add_node(node_id=0, dtb_path=str(dtb), kernel_path=str(kernel), extra_args=extra_args)
        await orchestrator.start()

        logger.info("Sending 'X' to QEMU RX...")
        frame = create_lin_frame(1_000_000, LinMessageType.LinMessageType.Data, b"X")
        await sim_transport.publish(rx_topic, frame)

        # Advance clock to process 'X'
        assert orchestrator.vta is not None
        await orchestrator.vta.step(5_000_000)

        logger.info("Sending Break to QEMU RX...")
        frame = create_lin_frame(6_000_000, LinMessageType.LinMessageType.Break, None)
        await sim_transport.publish(rx_topic, frame)

        # Advance clock to process Break
        assert orchestrator.vta is not None
        await orchestrator.vta.step(5_000_000)

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
            assert orchestrator.vta is not None
            await orchestrator.vta.step(5_000_000)

        assert found_x, f"Failed to receive Echo for 'X', received: {received}"
        assert found_b, f"Failed to receive Echo for Break, received: {received}"

        logger.info("SUCCESS: LIN UART verified.")

    finally:
        shutil.rmtree(tmpdir)
