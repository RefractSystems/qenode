"""
SOTA Test Module: test_lin_multi_node

Context:
This module implements tests for the test_lin_multi_node subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_lin_multi_node.
"""

from __future__ import annotations

import asyncio
import hashlib
import logging
import shutil
from collections.abc import Callable
from pathlib import Path
from typing import TYPE_CHECKING

import pytest

if TYPE_CHECKING:
    import zenoh

    from tools.testing.virtmcu_test_suite.process import AsyncManagedProcess


from virtmcu.lin import LinFrame, LinMessageType

from tools.testing.virtmcu_test_suite.factory import compile_dtb, compile_firmware
from tools.testing.virtmcu_test_suite.orchestrator import SimulationOrchestrator

logger = logging.getLogger(__name__)


@pytest.mark.asyncio
async def test_multi_node_lin(
    zenoh_router: str, qemu_launcher: Callable[..., AsyncManagedProcess], zenoh_session: zenoh.Session, tmp_path: Path
) -> None:
    tmpdir = tmp_path

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

    unique_id = hashlib.sha256(tmp_path.name.encode()).hexdigest()[:8]
    lin_topic = f"{unique_id}/sim/lin"

    # Generate world YAML for coordinator
    world_yaml = tmp_path / "world.yml"
    world_yaml.write_text("""
peripherals:
  - name: "0"
    type: LIN
  - name: "1"
    type: LIN

topology:
  links:
    - { type: "lin", nodes: ["0", "1"] }
""")

    # Generate Master DTB in tmpdir
    master_dtb = Path(tmpdir) / "lin_master.dtb"  # Replace router and compile
    compile_dtb(
        Path("tests/fixtures/guest_apps/lin_bridge/lin_test.dts"),
        {"ZENOH_ROUTER_ENDPOINT": router_endpoint, '"sim/lin"': f'"{lin_topic}"'},
        master_dtb,
    )

    from tools.testing.utils import get_time_multiplier

    stall_timeout = int(1000 * get_time_multiplier())

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
        "-device",
        f"virtmcu-clock,mode=slaved-icount,node=0,router={router_endpoint},stall-timeout={stall_timeout},coordinated=true",
    ]

    # Generate Slave DTB
    slave_dtb = Path(tmpdir) / "lin_slave.dtb"
    compile_dtb(
        Path("tests/fixtures/guest_apps/lin_bridge/lin_test.dts"),
        {
            "node = <0>;": "node = <1>;",
            "ZENOH_ROUTER_ENDPOINT": router_endpoint,
            '"sim/lin"': f'"{lin_topic}"',
            'topic = "sim/lin";': 'topic = "sim/lin"; debug;',
        },
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
        "-device",
        f"virtmcu-clock,mode=slaved-icount,node=1,router={router_endpoint},stall-timeout={stall_timeout},coordinated=true",
    ]

    # 3. Connect to Zenoh
    session = zenoh_session

    bus_messages: list[tuple[str, int, bytes]] = []

    def on_bus_msg(sample: zenoh.Sample) -> None:
        try:
            payload = sample.payload.to_bytes()
            frame = LinFrame.LinFrame.GetRootAsLinFrame(payload, 0)
            msg_type = frame.Type()
            data_len = frame.DataLength()
            data = bytes([frame.Data(i) for i in range(data_len)])
            topic = str(sample.key_expr)
            logger.info(f"Bus: {topic} type={msg_type} data={data!r}")
            bus_messages.append((topic, msg_type, data))
        except Exception as e:  # noqa: BLE001
            # Subscription callbacks should not crash the transport thread on malformed payloads
            logger.error(f"Error decoding message: {e}")

    # Listen to both nodes' TX
    sub0 = await asyncio.to_thread(lambda: session.declare_subscriber(f"{lin_topic}/0/tx", on_bus_msg))
    sub1 = await asyncio.to_thread(lambda: session.declare_subscriber(f"{lin_topic}/1/tx", on_bus_msg))

    import subprocess

    from tools.testing.virtmcu_test_suite.conftest_core import wait_for_zenoh_discovery

    # Start coordinator manually
    coord_cmd = [
        str(Path.cwd() / "target/release/zenoh_coordinator"),
        "--topology",
        str(world_yaml),
        "--connect",
        router_endpoint,
        "--pdes",
        "--nodes",
        "2",
    ]

    # Ensure it's built
    if not (Path.cwd() / "target/release/zenoh_coordinator").exists():
        subprocess.run([shutil.which("cargo") or "cargo", "build", "--release", "-p", "zenoh_coordinator"], check=True)

    coord_proc = await asyncio.create_subprocess_exec(
        *coord_cmd,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
    )

    async def _stream_coord_output(stream: asyncio.StreamReader, name: str) -> None:
        while True:
            line = await stream.readline()
            if not line:
                break
            logger.info(f"Coordinator {name}: {line.decode().strip()}")

    [
        asyncio.create_task(_stream_coord_output(coord_proc.stdout, "STDOUT")),  # type: ignore[arg-type]
        asyncio.create_task(_stream_coord_output(coord_proc.stderr, "STDERR")),  # type: ignore[arg-type]
    ]

    try:
        await wait_for_zenoh_discovery(session, "sim/coordinator/liveliness")

        async with SimulationOrchestrator(session, router_endpoint, qemu_launcher) as sim:
            logger.info("Launching Master and Slave via Orchestrator...")
            sim.add_node(node_id=0, dtb_path=str(master_dtb), kernel_path=str(master_kernel), extra_args=master_args)
            sim.add_node(node_id=1, dtb_path=str(slave_dtb), kernel_path=str(slave_kernel), extra_args=slave_args)

            await sim.start()

            def condition_met() -> bool:
                found_master_header = False
                found_slave_response = False
                for topic, msg_type, data in bus_messages:
                    if topic.endswith("/0/tx") and msg_type == LinMessageType.LinMessageType.Break:
                        found_master_header = True
                    if topic.endswith("/1/tx") and msg_type == LinMessageType.LinMessageType.Data and b"S" in data:
                        found_slave_response = True
                return found_master_header and found_slave_response

            await sim.run_until(condition_met, timeout=120.0, step_ns=10_000_000)

            logger.info(f"SUCCESS: Multi-node LIN communication verified at vtime={sim._vtime_ns}!")
    finally:
        if coord_proc.returncode is None:
            coord_proc.terminate()
            await coord_proc.wait()

    await asyncio.to_thread(sub0.undeclare)
    await asyncio.to_thread(sub1.undeclare)
