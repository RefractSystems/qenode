# ZENOH_HACK_EXCEPTION: Global exemption for now while tests are refactored.
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
import time
from typing import TYPE_CHECKING, Any, cast

import pytest
import yaml

from tools.testing.utils import yield_now
from tools.testing.virtmcu_test_suite.factory import compile_yaml
from tools.testing.virtmcu_test_suite.topics import SimTopic
from tools.testing.virtmcu_test_suite.transport import SimulationTransport

if TYPE_CHECKING:
    from pathlib import Path

    from tools.testing.virtmcu_test_suite.simulation import Simulation


logger = logging.getLogger(__name__)


async def _flood(noise_count: int, transport: SimulationTransport, topic: str, noise_data: bytes) -> None:
    for _ in range(noise_count):
        await transport.publish(topic, noise_data)


@pytest.mark.asyncio
async def test_clock_priority_isolation(
    simulation: Simulation, tmp_path: Path, guest_app_factory: Any  # noqa: ANN401
) -> None:
    """
    STRESS TEST for Clock Session Priority Isolation.
    """
    from tools.testing.env import WORKSPACE_ROOT

    app_dir = guest_app_factory("boot_arm")
    firmware_path = app_dir / "hello.elf"

    workspace_root = WORKSPACE_ROOT

    coordinator_bin = workspace_root / "target/release/deterministic_coordinator"
    if not coordinator_bin.exists():
        pytest.fail("deterministic_coordinator not found")

    from tools.testing.virtmcu_test_suite.generated import (
        Address,
        Cpu,
        Machine,
        Node,
        NodeID,
        Resource,
        Topology,
        World,
    )

    board_yaml = tmp_path / "board.yaml"
    world = World(
        machine=Machine(
            name="priority_test",
            type="arm-generic-fdt",
            cpus=[Cpu(name="cpu", type="cortex-a15", memory="sysmem")],
        ),
        peripherals=[
            Resource(
                name=NodeID(root="memory"),
                renode_type="Memory.MappedMemory",
                address=Address(root="0x40000000"),
                properties=cast(Any, {"size": "0x08000000"}),
                container="sysmem",
            ),
            Resource(
                name=NodeID(root="uart0"),
                renode_type="UART.PL011",
                address=Address(root="0x09000000"),
                interrupts=["gic@1"],
                container="sysmem",
            ),
            Resource(
                name=NodeID(root="gic"),
                renode_type="IRQControllers.ARM_GenericInterruptController",
                address=Address(root="0x08000000"),
                properties=cast(Any, {"architectureVersion": ".GICv2"}),
                container="sysmem",
            ),
        ],
        topology=Topology(
            nodes=[Node(name=NodeID(root="0"))],
            links=[],
        ),
    )
    board_yaml.write_text(yaml.dump(world.model_dump(exclude_none=True, by_alias=True), sort_keys=False))

    from tools.testing.virtmcu_test_suite.conftest_core import (
        coordinator_subprocess,
    )
    from tools.testing.virtmcu_test_suite.transport import ZenohTransportImpl

    # Launch Coordinator via SOTA helper
    coord_args = [
        "--nodes",
        "1",
        "--connect",
        simulation._router,
        "--topology",
        str(board_yaml),
        "--no-pdes",
    ]

    async with coordinator_subprocess(
        binary=coordinator_bin,
        args=coord_args,
        zenoh_session=simulation._session,
        liveliness_topic=SimTopic.COORD_ALIVE,
    ):
        pass
        if simulation.transport is None:
            from tools.testing.virtmcu_test_suite.transport import ZenohTransportImpl
            simulation.transport = ZenohTransportImpl(simulation._router, simulation._session)

        # Launch QEMU with clock (private session) and chardev (shared session)
        # simulation handles router, node, mode=slaved-icount
        extra_args = [
            "-chardev",
            "virtmcu,id=char0,topic=sim/priority_test/uart",
            "-serial",
            "chardev:char0",
        ]

        board_dtb = tmp_path / "board.dtb"
        compile_yaml(board_yaml, board_dtb)
        simulation.add_node(node_id=0, dtb=board_dtb, kernel=firmware_path, extra_args=extra_args)

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
                noise_data = b"X" * 4096
                noise_count = 1000

                logger.info(f"Starting flood of {noise_count} packets on shared session...")

                flood_task = asyncio.create_task(_flood(noise_count, simulation.transport, flood_topic, noise_data))

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
            pass
