"""
SOTA Test Module: test_lin_stress

Context:
This module implements tests for the test_lin_stress subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_lin_stress.
"""

from __future__ import annotations
 
import hashlib
import logging
import os
from pathlib import Path
from typing import TYPE_CHECKING

import pytest

from tools.testing.virtmcu_test_suite.factory import compile_dtb, compile_firmware
from tools.testing.virtmcu_test_suite.topics import SimTopic

if TYPE_CHECKING:
    from tools.testing.virtmcu_test_suite.simulation import Simulation


import flatbuffers

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


@pytest.mark.asyncio
@pytest.mark.timeout(600)
async def test_lin_stress(
    simulation: Simulation, tmp_path: Path
) -> None:

    is_asan = os.environ.get("VIRTMCU_USE_ASAN") == "1"
    tmpdir = tmp_path
    router_endpoint = simulation._router

    # Build ELF
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

    logger.info(f"Starting QEMU with topic {lin_topic} using Simulation...")
    simulation.add_node(node_id=0, dtb=dtb, kernel=kernel, extra_args=extra_args)
    async with simulation as sim:
        if sim.transport is None:
            from tools.testing.virtmcu_test_suite.transport import ZenohTransportImpl
            sim.transport = ZenohTransportImpl(sim._router, sim._session)
        assert sim.transport is not None

        received_count = 0
        errors = 0

        def on_bus_msg(payload: bytes) -> None:
            nonlocal received_count, errors
            try:
                frame = LinFrame.LinFrame.GetRootAsLinFrame(payload, 0)
                # Count any data frame received on node 0 TX topic
                if frame.Type() == LinMessageType.LinMessageType.Data:
                    received_count += 1
            except Exception:  # noqa: BLE001
                errors += 1

        tx_topic = f"{lin_topic}/0/tx"
        rx_topic = f"{lin_topic}/0/rx"
        await sim.transport.subscribe(tx_topic, on_bus_msg)

        logger.info("Starting staggered frame injection...")
        step_ns = 1_000_000  # 1ms steps
        total_steps = 20 if is_asan else 100

        for i in range(total_steps):
            # Send one frame every ms
            frame = create_lin_frame(i * step_ns, LinMessageType.LinMessageType.Data, b"S")
            await sim.transport.publish(rx_topic, bytes(frame))

            # Advance clock by 1ms
            await sim.vta.step(step_ns)  # LINT_EXCEPTION: vta_step_loop

        logger.info(f"Received {received_count} echo responses, {errors} errors.")
        assert received_count > 0, "No responses received!"
        logger.info(f"SUCCESS: Received {received_count} responses.")
