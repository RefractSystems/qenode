"""
SOTA Test Module: test_lin_stress

Context:
This module implements tests for the test_lin_stress subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_lin_stress.
"""

import asyncio
import logging
import sys
from pathlib import Path

import pytest

from tools.testing.virtmcu_test_suite.factory import compile_dtb, compile_firmware

# Add tools/lin_fbs to sys.path
sys.path.append(str(Path.cwd() / "tools/lin_fbs"))

import flatbuffers
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
async def test_lin_stress(simulation, zenoh_router, zenoh_session):
    import shutil
    import tempfile

    tmpdir = tempfile.mkdtemp(prefix="virtmcu-lin-stress-")

    router_endpoint = zenoh_router

    # Build ELF
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
        {"ZENOH_ROUTER_ENDPOINT": router_endpoint, '"sim/lin"': f'"{lin_topic}"'},
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
        f"virtmcu-clock,mode=slaved-icount,node=0,router={router_endpoint}",
        # The s32k144-lpuart device is instantiated by the DTB, no need for -device
    ]

    # 2. Connect to Zenoh
    session = zenoh_session

    received_count = 0
    errors = 0

    def on_bus_msg(sample):
        nonlocal received_count, errors
        try:
            payload = sample.payload.to_bytes()
            frame = LinFrame.LinFrame.GetRootAsLinFrame(payload, 0)
            # Count any data frame received on node 0 TX topic
            if frame.Type() == LinMessageType.LinMessageType.Data:
                received_count += 1
        except Exception:
            errors += 1

    # Listen to Node 0 TX and publish to Node 0 RX
    # Note: s32k144-lpuart uses {topic}/{node_id}/tx and {topic}/{node_id}/rx
    tx_topic = f"{lin_topic}/0/tx"
    rx_topic = f"{lin_topic}/0/rx"
    sub = await asyncio.to_thread(lambda: session.declare_subscriber(tx_topic, on_bus_msg))
    pub = await asyncio.to_thread(lambda: session.declare_publisher(rx_topic))

    logger.info(f"Starting QEMU with topic {lin_topic} using VirtmcuSimulation...")
    async with await simulation(dtb, kernel, extra_args=extra_args, ignore_clock_check=True) as sim:
        try:
            logger.info("Starting staggered frame injection...")
            step_ns = 1_000_000  # 1ms steps
            total_steps = 100

            for i in range(total_steps):
                # Send one frame every ms
                frame = create_lin_frame(i * step_ns, LinMessageType.LinMessageType.Data, b"S")
                from functools import partial

                await asyncio.to_thread(partial(pub.put, frame))

                # Advance clock by 1ms
                await sim.vta.step(step_ns)

            logger.info(f"Received {received_count} echo responses, {errors} errors.")
            assert received_count > 0, "No responses received!"
            logger.info(f"SUCCESS: Received {received_count} responses.")

        finally:
            await asyncio.to_thread(sub.undeclare)
            shutil.rmtree(tmpdir)
