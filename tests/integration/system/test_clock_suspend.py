"""
SOTA Test Module: test_clock_suspend

Context:
This module implements tests for the test_clock_suspend subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_clock_suspend.
"""

import asyncio
import logging
import subprocess
from functools import partial

import pytest
import vproto

logger = logging.getLogger(__name__)


def build_clock_suspend_artifacts():
    from tools.testing.env import WORKSPACE_ROOT

    workspace_root = WORKSPACE_ROOT
    dtb_path = workspace_root / "tests/fixtures/guest_apps/boot_arm/minimal.dtb"
    kernel_path = workspace_root / "tests/fixtures/guest_apps/boot_arm/hello.elf"

    if not dtb_path.exists() or not kernel_path.exists():
        subprocess.run(["make", "-C", "tests/fixtures/guest_apps/boot_arm", "all"], check=True)

    return dtb_path, kernel_path


@pytest.mark.asyncio
async def test_clock_suspend(simulation):
    """
    clock in slaved-suspend mode.
    Verify that virtual time advances and matches queries.
    """
    dtb, kernel = build_clock_suspend_artifacts()
    extra_args = ["-device", "virtmcu-clock,node=0,mode=slaved-suspend"]

    async with await simulation(dtb, kernel, nodes=[0], extra_args=extra_args, ignore_clock_check=True) as sim:
        # 1. Initial sync should succeed
        vtime = (await sim.vta.step(1_000_000))[0]
        assert vtime >= 1_000_000

        # 2. Advance clock significantly
        vtime = (await sim.vta.step(100_000_000))[0]
        assert vtime >= 101_000_000


@pytest.mark.asyncio
@pytest.mark.skip_asan
async def test_clock_stall(simulation):
    """
    clock stall detection.
    """
    dtb, kernel = build_clock_suspend_artifacts()

    # Use a shorter stall-timeout specifically for the stall test.
    stall_timeout = 2000
    extra_args = [
        "-device",
        f"virtmcu-clock,node=0,mode=slaved-suspend,stall-timeout={stall_timeout}",
    ]

    async with await simulation(dtb, kernel, nodes=[0], extra_args=extra_args, ignore_clock_check=True) as sim:
        # Trigger stall by pausing emulation
        await sim.bridge.pause_emulation()

        try:
            with pytest.raises(RuntimeError, match="reported CLOCK STALL"):
                # Wait longer than stall_timeout to ensure it's triggered
                await sim.vta.step(10_000_000, timeout=(stall_timeout / 1000.0) + 10.0)

            await sim.bridge.start_emulation()
            # Give QEMU a moment to resume
            vtime = (await sim.vta.step(1_000_000))[0]
            assert vtime > 0

        finally:
            try:
                await asyncio.wait_for(sim.bridge.start_emulation(), timeout=2.0)
            except Exception as e:
                logger.error(f"Failed to start emulation in finally: {e}")


@pytest.mark.asyncio
async def test_slow_boot_fast_execute(zenoh_router, qemu_launcher, zenoh_session):
    """
    Verify "slow boot / fast execute" invariant.
    The first quantum (initial sync) should survive a delay longer than the standard stall-timeout.
    Subsequent quantums should stall if delayed.
    """
    dtb_path, kernel_path = build_clock_suspend_artifacts()

    # We must use a short stall-timeout to avoid sleeping for 5 minutes during ASan.
    # However, ASan can be slow enough that 2000ms is too tight for normal execution.
    # We will use 5000ms.
    stall_timeout = 5000
    extra_args = [
        "-S",
        "-device",
        f"virtmcu-clock,node=0,mode=slaved-suspend,router={zenoh_router},stall-timeout={stall_timeout}",
    ]

    bridge = await qemu_launcher(dtb_path, kernel_path, extra_args=extra_args, ignore_clock_check=True)

    from tests.conftest import VirtualTimeAuthority
    from tools.testing.virtmcu_test_suite.conftest_core import VirtmcuSimulation

    vta = VirtualTimeAuthority(zenoh_session, [0])
    sim = VirtmcuSimulation(bridge, vta, init_barrier=False)

    async with sim:
        # 2. Delay the first sync beyond stall_timeout but within boot timeout (10m)
        # SLEEP_EXCEPTION: testing wall-clock boundaries for boot vs stall timeouts
        await asyncio.sleep(6.0)  # SLEEP_EXCEPTION: testing wall-clock boundaries

        # This should succeed despite the 6s delay because it's the first quantum
        await vta.init(timeout=10.0)

        # 3. Advance clock to complete the first quantum
        await vta.step(1_000_000, timeout=10.0)

        # Subsequent quantum should be subject to the strict 5s stall-timeout
        # SLEEP_EXCEPTION: testing wall-clock boundaries for boot vs stall timeouts
        await asyncio.sleep(6.0)  # SLEEP_EXCEPTION: testing wall-clock boundaries
        try:
            assert sim.bridge is not None
            if sim.bridge._watchdog_task:
                sim.bridge._watchdog_task.cancel()
            with pytest.raises(RuntimeError, match="reported CLOCK STALL"):
                # This should stall
                await vta.step(1_000_000, timeout=10.0)
        finally:
            pass


@pytest.mark.asyncio
async def test_netdev(simulation):
    """
    netdev basic packet delivery.
    """
    dtb, kernel = build_clock_suspend_artifacts()

    extra_args = [
        "-icount",
        "shift=0,align=off,sleep=off",
        "-device",
        "virtmcu-clock,node=1,mode=slaved-icount",
        "-netdev",
        "virtmcu,node=1,id=n1",
    ]
    async with await simulation(dtb, kernel, nodes=[1], extra_args=extra_args, ignore_clock_check=True) as sim:
        NETDEV_TOPIC = "sim/eth/frame/1/rx"  # noqa: N806
        DELIVERY_VTIME_NS = 500_000  # noqa: N806
        FRAME = b"\xff" * 14  # noqa: N806
        packet = vproto.ZenohFrameHeader(DELIVERY_VTIME_NS, 0, len(FRAME)).pack() + FRAME

        pub = await asyncio.to_thread(lambda: sim.vta.session.declare_publisher(NETDEV_TOPIC))
        await asyncio.to_thread(lambda: pub.put(packet))

        await sim.vta.step(1_000_000)
        assert sim.vta.current_vtimes[1] >= 1_000_000


@pytest.mark.asyncio
async def test_netdev_stress(simulation):
    """
    netdev stress test.
    """
    dtb, kernel = build_clock_suspend_artifacts()
    extra_args = [
        "-icount",
        "shift=0,align=off,sleep=off",
        "-device",
        "virtmcu-clock,node=0,mode=slaved-icount",
        "-netdev",
        "virtmcu,node=0,id=n0",
    ]
    async with await simulation(dtb, kernel, nodes=[0], extra_args=extra_args, ignore_clock_check=True) as sim:
        for i in range(100, 0, -1):
            vtime = i * 1_000_000
            packet = vproto.ZenohFrameHeader(vtime, 0, 14).pack() + b"\xee" * 14
            await asyncio.to_thread(partial(sim.vta.session.put, "sim/eth/frame/0/rx", packet))

        await sim.vta.step(200_000_000)
        assert sim.vta.current_vtimes[0] >= 200_000_000


@pytest.mark.asyncio
async def test_determinism(simulation):
    """
    Clock/Netdev determinism.
    """
    dtb, kernel = build_clock_suspend_artifacts()
    extra_args = [
        "-icount",
        "shift=0,align=off,sleep=off",
        "-device",
        "virtmcu-clock,node=0,mode=slaved-icount",
        "-netdev",
        "virtmcu,node=0,id=n0",
    ]
    async with await simulation(dtb, kernel, nodes=[0], extra_args=extra_args, ignore_clock_check=True) as sim:
        packet = vproto.ZenohFrameHeader(5_000_000, 0, 14).pack() + b"\xdd" * 14
        await asyncio.to_thread(lambda: sim.vta.session.put("sim/eth/frame/0/rx", packet))

        await sim.vta.step(10_000_000)
        assert sim.vta.current_vtimes[0] >= 10_000_000
