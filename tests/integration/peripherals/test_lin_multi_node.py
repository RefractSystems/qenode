"""
SOTA Test Module: test_lin_multi_node

Context:
This module implements tests for the test_lin_multi_node subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_lin_multi_node.
"""

import asyncio
import logging
import sys
from pathlib import Path

import pytest

# Add tools/lin_fbs to sys.path
sys.path.append(str(Path.cwd() / "tools/lin_fbs"))

from virtmcu.lin import LinFrame, LinMessageType

from tools.testing.virtmcu_test_suite.factory import compile_dtb, compile_firmware
from tools.testing.virtmcu_test_suite.orchestrator import SimulationOrchestrator

logger = logging.getLogger(__name__)


@pytest.mark.asyncio
async def test_multi_node_lin(zenoh_router, qemu_launcher, zenoh_coordinator, zenoh_session):  # noqa: ARG001
    import shutil
    import tempfile

    tmpdir = tempfile.mkdtemp(prefix="virtmcu-lin-multi-")

    router_endpoint = zenoh_router

    # 1. Build Master/Slave ELFs
    master_kernel = Path(tmpdir) / "lin_master.elf"
    slave_kernel = Path(tmpdir) / "lin_slave.elf"

    compile_firmware(
        [Path("tests/fixtures/guest_apps/lin_bridge/lin_master.S")],
        master_kernel,
        linker_script=Path("tests/fixtures/guest_apps/lin_bridge/lin_echo.ld"),
    )
    compile_firmware(
        [Path("tests/fixtures/guest_apps/lin_bridge/lin_slave.S")],
        slave_kernel,
        linker_script=Path("tests/fixtures/guest_apps/lin_bridge/lin_echo.ld"),
    )

    # Use unique topic to avoid interference
    import uuid

    unique_id = str(uuid.uuid4())[:8]
    lin_topic = f"sim/lin/{unique_id}"

    # Generate Master DTB in tmpdir
    master_dtb = Path(tmpdir) / "lin_master.dtb"  # Replace router and compile
    compile_dtb(
        Path("tests/fixtures/guest_apps/lin_bridge/lin_test.dts"),
        {"ZENOH_ROUTER_ENDPOINT": router_endpoint, '"sim/lin"': f'"{lin_topic}"'},
        master_dtb,
    )

    # Master node (Node 0)
    master_args = [
        "-cpu",
        "cortex-a15",
        "-chardev",
        "null,id=n0",
        "-serial",
        "chardev:n0",
        "-net",
        "none",
    ]

    # Generate Slave DTB
    slave_dtb = Path(tmpdir) / "lin_slave.dtb"
    compile_dtb(
        Path("tests/fixtures/guest_apps/lin_bridge/lin_test.dts"),
        {"node = <0>;": "node = <1>;", "ZENOH_ROUTER_ENDPOINT": router_endpoint, '"sim/lin"': f'"{lin_topic}"'},
        slave_dtb,
    )

    slave_args = [
        "-cpu",
        "cortex-a15",
        "-chardev",
        "null,id=n1",
        "-serial",
        "chardev:n1",
        "-net",
        "none",
    ]

    # 3. Connect to Zenoh
    session = zenoh_session

    bus_messages = []

    def on_bus_msg(sample):
        try:
            payload = sample.payload.to_bytes()
            frame = LinFrame.LinFrame.GetRootAsLinFrame(payload, 0)
            msg_type = frame.Type()
            data_len = frame.DataLength()
            data = bytes([frame.Data(i) for i in range(data_len)])
            topic = str(sample.key_expr)
            logger.info(f"Bus: {topic} type={msg_type} data={data!r}")
            bus_messages.append((topic, msg_type, data))
        except Exception as e:
            logger.error(f"Error: {e}")

    # Listen to both nodes' TX
    sub0 = await asyncio.to_thread(lambda: session.declare_subscriber(f"{lin_topic}/0/tx", on_bus_msg))
    sub1 = await asyncio.to_thread(lambda: session.declare_subscriber(f"{lin_topic}/1/tx", on_bus_msg))

    try:
        async with SimulationOrchestrator(session, router_endpoint, qemu_launcher) as sim:
            logger.info("Launching Master and Slave via Orchestrator...")
            sim.add_node(node_id=0, dtb_path=str(master_dtb), kernel_path=str(master_kernel), extra_args=master_args)
            sim.add_node(node_id=1, dtb_path=str(slave_dtb), kernel_path=str(slave_kernel), extra_args=slave_args)

            await sim.start()

            def condition_met():
                found_master_header = False
                found_slave_response = False
                for topic, msg_type, data in bus_messages:
                    if topic == f"{lin_topic}/0/tx" and msg_type == LinMessageType.LinMessageType.Break:
                        found_master_header = True
                    if topic == f"{lin_topic}/1/tx" and msg_type == LinMessageType.LinMessageType.Data and b"S" in data:
                        found_slave_response = True
                return found_master_header and found_slave_response

            await sim.run_until(condition_met, timeout=20.0, step_ns=1_000_000)

            logger.info(f"SUCCESS: Multi-node LIN communication verified at vtime={sim._vtime_ns}!")

    finally:
        await asyncio.to_thread(sub0.undeclare)
        await asyncio.to_thread(sub1.undeclare)
        shutil.rmtree(tmpdir)
