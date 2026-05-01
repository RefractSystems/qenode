"""
SOTA Test Module: test_priority

Context:
This module implements tests for the test_priority subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_priority.
"""

import asyncio
import logging
import time

import pytest
import zenoh

from tests.conftest import VirtualTimeAuthority
from tools.testing.utils import yield_now

logger = logging.getLogger(__name__)


@pytest.mark.asyncio
async def test_clock_priority_isolation(zenoh_router, zenoh_session, qemu_launcher, tmp_path):
    """
    STRESS TEST for Clock Session Priority Isolation.
    """
    from tools.testing.env import WORKSPACE_ROOT

    workspace_root = WORKSPACE_ROOT
    firmware_path = workspace_root / "tests/fixtures/guest_apps/boot_arm/hello.elf"
    if not firmware_path.exists():
        import subprocess

        subprocess.run(["make", "-C", "tests/fixtures/guest_apps/boot_arm", "hello.elf"], check=True)

    coordinator_bin = workspace_root / "target/release/deterministic_coordinator"
    if not coordinator_bin.exists():
        pytest.fail("deterministic_coordinator not found")

    board_yaml = tmp_path / "board.yaml"
    board_yaml.write_text("""
machine:
  name: priority_test
  type: arm-generic-fdt
  cpus:
  - name: cpu
    type: cortex-a15
    memory: sysmem
peripherals:
- name: memory
  renode_type: Memory.MappedMemory
  address: '0x40000000'
  properties:
    size: '0x08000000'
  container: sysmem
- name: uart0
  renode_type: UART.PL011
  address: '0x09000000'
  interrupts:
  - gic@1
  container: sysmem
- name: gic
  renode_type: IRQControllers.ARM_GenericInterruptController
  address: '0x08000000'
  properties:
    architectureVersion: .GICv2
  container: sysmem

topology:
  nodes:
    - id: 0
      name: node0
  links: []
""")

    # Launch Coordinator
    coord_proc = await asyncio.create_subprocess_exec(
        str(coordinator_bin),
        "--nodes",
        "1",
        "--connect",
        zenoh_router,
        "--topology",
        str(board_yaml),
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
    )

    # Launch QEMU with clock (private session) and chardev (shared session)
    extra_args = [
        "-S",
        "-icount",
        "shift=0,align=off,sleep=off",
        "-device",
        f"virtmcu-clock,node=0,mode=slaved-icount,router={zenoh_router}",
        "-chardev",
        f"virtmcu,id=char0,node=0,router={zenoh_router},topic=sim/priority_test/uart",
        "-serial",
        "chardev:char0",
    ]

    from tools.testing.virtmcu_test_suite.conftest_core import VirtmcuSimulation

    bridge = await qemu_launcher(str(board_yaml), firmware_path, ignore_clock_check=True, extra_args=extra_args)

    ta_session = await asyncio.to_thread(lambda: zenoh.open(zenoh.Config()))
    vta = VirtualTimeAuthority(ta_session, node_ids=[0])

    async with VirtmcuSimulation(bridge, vta):
        try:
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

            async def flood():
                for _ in range(noise_count):
                    await asyncio.to_thread(lambda: pub.put(noise_data))

            flood_task = asyncio.create_task(flood())

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
            assert avg_stress < 0.100, f"Clock synchronization starved! RTT={avg_stress * 1000:.2f}ms"

            logger.info(f"Clock Jitter Increase: {(avg_stress - avg_baseline) * 1000:.2f} ms")

        finally:
            await asyncio.to_thread(ta_session.close)
        await bridge.close()
        coord_proc.terminate()
        await coord_proc.wait()
