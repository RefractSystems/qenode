"""
SOTA Test Module: test_priority

Context:
This module implements tests for the test_priority subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_priority.
"""

from __future__ import annotations

import asyncio
import logging
import shutil
import time
from typing import TYPE_CHECKING

import pytest
import zenoh

from tools.testing.utils import yield_now
from tools.testing.virtmcu_test_suite.topics import SimTopic

if TYPE_CHECKING:
    from pathlib import Path

    from tools.testing.virtmcu_test_suite.simulation import Simulation


logger = logging.getLogger(__name__)


async def _flood(noise_count: int, pub: zenoh.Publisher, noise_data: bytes) -> None:
    for _ in range(noise_count):
        await asyncio.to_thread(lambda: pub.put(noise_data))


@pytest.mark.asyncio
async def test_clock_priority_isolation(
    zenoh_router: str, zenoh_session: zenoh.Session, simulation: Simulation, tmp_path: Path
) -> None:
    """
    STRESS TEST for Clock Session Priority Isolation.
    """
    from tools.testing.env import WORKSPACE_ROOT

    workspace_root = WORKSPACE_ROOT
    firmware_path = workspace_root / "tests/fixtures/guest_apps/boot_arm/hello.elf"
    if not firmware_path.exists():
        import subprocess

        subprocess.run(
            [shutil.which("make") or "make", "-C", "tests/fixtures/guest_apps/boot_arm", "hello.elf"], check=True
        )

    coordinator_bin = workspace_root / "target/release/deterministic_coordinator"
    if not coordinator_bin.exists():
        pytest.fail("deterministic_coordinator not found")

    from tools.testing.virtmcu_test_suite.world_schema import (
        MachineSpec,
        NodeSpec,
        TopologySpec,
        WorldYaml,
    )

    board_yaml = tmp_path / "board.yaml"
    world = WorldYaml(
        machine=MachineSpec(
            name="priority_test",
            type="arm-generic-fdt",
            cpus=[{"name": "cpu", "type": "cortex-a15", "memory": "sysmem"}],
        ),
        peripherals=[
            {
                "name": "memory",
                "renode_type": "Memory.MappedMemory",
                "address": "0x40000000",
                "properties": {"size": "0x08000000"},
                "container": "sysmem",
            },
            {
                "name": "uart0",
                "renode_type": "UART.PL011",
                "address": "0x09000000",
                "interrupts": ["gic@1"],
                "container": "sysmem",
            },
            {
                "name": "gic",
                "renode_type": "IRQControllers.ARM_GenericInterruptController",
                "address": "0x08000000",
                "properties": {"architectureVersion": ".GICv2"},
                "container": "sysmem",
            },
        ],
        topology=TopologySpec(
            nodes=[NodeSpec(name="0")],
            links=[],
        ),
    )
    board_yaml.write_text(world.to_yaml())

    from tools.testing.virtmcu_test_suite.conftest_core import (
        coordinator_subprocess,
        open_client_session,
    )
    from tools.testing.virtmcu_test_suite.transport import ZenohTransportImpl

    # Launch Coordinator via SOTA helper
    coord_args = [
        "--nodes",
        "1",
        "--connect",
        zenoh_router,
        "--topology",
        str(board_yaml),
        "--no-pdes",
    ]

    async with coordinator_subprocess(
        binary=coordinator_bin,
        args=coord_args,
        zenoh_session=zenoh_session,
        liveliness_topic=SimTopic.COORD_ALIVE,
    ):
        ta_session = await asyncio.to_thread(lambda: open_client_session(connect=zenoh_router))
        simulation.transport = ZenohTransportImpl(zenoh_router, ta_session)

        # Launch QEMU with clock (private session) and chardev (shared session)
        # simulation handles router, node, mode=slaved-icount
        extra_args = [
            "-chardev",
            "virtmcu,id=char0,topic=sim/priority_test/uart",
            "-serial",
            "chardev:char0",
        ]

        simulation.add_node(node_id=0, dtb=board_yaml, kernel=firmware_path, extra_args=extra_args)

        try:
            async with simulation as sim:
                vta = sim.vta
                assert vta is not None
                # Baseline: Measure RTT with NO load
                rtts_baseline = []
                logger.info("\nMeasuring baseline RTT (10ms quanta)...")
                for i in range(10):
                    t0 = time.perf_counter()
                    await vta.step(delta_ns=10_000_000)
                    rtts_baseline.append(time.perf_counter() - t0)
                    if i % 2 == 0:
                        logger.info(f"  Baseline {i}: {rtts_baseline[-1] * 1000:.2f} ms")

                avg_baseline = sum(rtts_baseline) / len(rtts_baseline)
                logger.info(f"Baseline Clock RTT: {avg_baseline * 1000:.2f} ms")

                # Stress: Flood the SHARED session
                flood_topic = "virtmcu/stress/noise"
                pub = await asyncio.to_thread(lambda: zenoh_session.declare_publisher(flood_topic))
                noise_data = b"X" * 4096
                noise_count = 1000

                logger.info(f"Starting flood of {noise_count} packets on shared session...")

                flood_task = asyncio.create_task(_flood(noise_count, pub, noise_data))

                # Measure RTT WITH load
                rtts_stress = []
                logger.info("Measuring stress RTT (10ms quanta)...")
                for i in range(10):
                    t0 = time.perf_counter()
                    await vta.step(delta_ns=10_000_000)
                    rtts_stress.append(time.perf_counter() - t0)
                    if i % 2 == 0:
                        logger.info(f"  Stress {i}: {rtts_stress[-1] * 1000:.2f} ms")
                    await yield_now()

                await flood_task
                avg_stress = sum(rtts_stress) / len(rtts_stress)
                logger.info(f"Stress Clock RTT: {avg_stress * 1000:.2f} ms")

                # Isolation ensures separate executors, so data plane flood shouldn't block clock.
                assert (
                    avg_stress < 0.200  # Relaxed for slow CI containers
                ), f"Clock synchronization starved! RTT={avg_stress * 1000:.2f}ms"

                logger.info(f"Clock Jitter Increase: {(avg_stress - avg_baseline) * 1000:.2f} ms")

        finally:
            await asyncio.to_thread(ta_session.close)
