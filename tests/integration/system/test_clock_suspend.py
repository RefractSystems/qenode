"""
SOTA Test Module: test_clock_suspend

Context:
This module implements tests for the test_clock_suspend subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_clock_suspend.
"""

from __future__ import annotations

import asyncio
import logging
import shutil
import subprocess
from pathlib import Path
from typing import TYPE_CHECKING

import pytest

from tools.testing.env import WORKSPACE_ROOT
from tools.testing.utils import get_time_multiplier

if TYPE_CHECKING:
    from tools.testing.virtmcu_test_suite.simulation import Simulation


logger = logging.getLogger(__name__)


def build_clock_suspend_artifacts() -> tuple[Path, Path]:
    workspace_root = WORKSPACE_ROOT
    dtb_path = workspace_root / "tests/fixtures/guest_apps/boot_arm/minimal.dtb"
    kernel_path = workspace_root / "tests/fixtures/guest_apps/boot_arm/hello.elf"

    if not dtb_path.exists() or not kernel_path.exists():
        subprocess.run(
            [shutil.which("make") or "make", "-C", "tests/fixtures/guest_apps/boot_arm", "all"],
            check=True,
            cwd=workspace_root,
        )

    return dtb_path, kernel_path


@pytest.mark.asyncio
async def test_clock_slaved_suspend_smoke(simulation: Simulation) -> None:
    """
    Verify basic clock advancement in slaved-suspend mode.
    """
    dtb, kernel = build_clock_suspend_artifacts()
    extra_args = ["-device", "virtmcu-clock,node=0,mode=slaved-suspend"]

    simulation.add_node(node_id=0, dtb=dtb, kernel=kernel, extra_args=extra_args)
    async with simulation as sim:
        # 1. Initial vtime should be small (we allow some slack for boot if not using icount)
        vtime = (await sim.vta.step(0))[0]
        # In slaved-suspend without icount, vtime is real-time. simulation fixture has some sleep(0.5) calls.
        assert vtime < 2_000_000_000 * get_time_multiplier()

        # 2. Advance 10ms
        vtime = (await sim.vta.step(10_000_000))[0]
        assert vtime >= 10_000_000

        # 3. Advance another 10ms
        vtime = (await sim.vta.step(10_000_000))[0]
        assert vtime >= 20_000_000


@pytest.mark.asyncio
async def test_clock_stall_detection(simulation: Simulation) -> None:
    """
    Verify that slaved-suspend mode correctly triggers and reports
    clock stall detection.
    """
    dtb, kernel = build_clock_suspend_artifacts()

    # Use a shorter stall-timeout specifically for the stall test, but scale it for the environment.
    base_stall = 2000
    stall_timeout = int(base_stall * get_time_multiplier())
    extra_args = [
        "-device",
        f"virtmcu-clock,node=0,mode=slaved-suspend,stall-timeout={stall_timeout}",
    ]

    simulation.add_node(node_id=0, dtb=dtb, kernel=kernel, extra_args=extra_args)
    async with simulation as sim:
        assert sim.bridge is not None
        # Trigger stall by pausing emulation
        await sim.bridge.pause_emulation()

        try:
            with pytest.raises(RuntimeError, match="reported CLOCK STALL"):
                # Wait longer than stall_timeout to ensure it's triggered.
                # vta.step already scales its timeout argument internally by get_time_multiplier().
                await sim.vta.step(10_000_000, timeout=(base_stall / 1000.0) + 10.0)

            assert sim.bridge is not None
            await sim.bridge.start_emulation()
            # Give QEMU a moment to resume
            vtime = (await sim.vta.step(1_000_000))[0]
            assert vtime > 0

        finally:
            if sim.bridge is not None:
                try:
                    await asyncio.wait_for(sim.bridge.start_emulation(), timeout=2.0 * get_time_multiplier())
                except Exception as e:  # noqa: BLE001
                    logger.error(f"Failed to start emulation in finally: {e}")


@pytest.mark.asyncio
async def test_slow_boot_fast_execute(simulation: Simulation) -> None:
    """
    Verify "slow boot / fast execute" invariant.
    The first quantum (initial sync) should survive a delay longer than the standard stall-timeout.
    Subsequent quantums should stall if delayed.
    """
    dtb, kernel = build_clock_suspend_artifacts()

    # 1. Start QEMU with a short stall timeout
    base_stall = 500
    stall_timeout = int(base_stall * get_time_multiplier())
    extra_args = [
        "-device",
        f"virtmcu-clock,node=0,mode=slaved-icount,stall-timeout={stall_timeout}",
    ]

    # Use init_barrier=False so we can manually wait BEFORE the first sync.
    simulation.add_node(node_id=0, dtb=dtb, kernel=kernel, extra_args=extra_args)
    simulation._init_barrier = False
    async with simulation as sim:
        # 2. Wait longer than stall_timeout BEFORE the first sync.
        #    The initial handshake/sync should NOT stall.
        await asyncio.sleep(1.0 * get_time_multiplier())  # SLEEP_EXCEPTION: deliberate delay to test boot grace period

        await sim.vta.init()

        # 3. First step should work
        await sim.vta.step(1_000_000)

        # 4. Now pause and step -> should stall
        assert sim.bridge is not None
        await sim.bridge.pause_emulation()
        with pytest.raises(RuntimeError, match="reported CLOCK STALL"):
            await sim.vta.step(1_000_000, timeout=(base_stall / 1000.0) + 5.0)


@pytest.mark.asyncio
async def test_clock_suspend_wfi(simulation: Simulation) -> None:
    """
    Verify that clock continues to advance during WFI in slaved-suspend mode.
    The test kernel performs a 10ms WFI.
    """
    workspace_root = WORKSPACE_ROOT
    dtb = workspace_root / "tests/fixtures/guest_apps/boot_arm/minimal.dtb"
    kernel = workspace_root / "tests/fixtures/guest_apps/telemetry_wfi/test_wfi.elf"

    if not kernel.exists():
        subprocess.run(
            [shutil.which("make") or "make", "-C", "tests/fixtures/guest_apps/telemetry_wfi", "all"],
            check=True,
            cwd=workspace_root,
        )

    extra_args = ["-device", "virtmcu-clock,node=0,mode=slaved-suspend"]

    simulation.add_node(node_id=0, dtb=dtb, kernel=kernel, extra_args=extra_args)
    async with simulation as sim:
        # Initial sync
        await sim.vta.step(0)

        # Step 20ms. The guest should be in WFI most of this time.
        vtime = (await sim.vta.step(20_000_000))[0]
        assert vtime >= 20_000_000

        # Verify UART output indicates WFI was reached
        assert sim.bridge is not None
        await sim.bridge.wait_for_line_on_uart("WFI started", timeout=5.0)


@pytest.mark.asyncio
async def test_clock_suspend_vtime_alignment(simulation: Simulation) -> None:
    """
    Verify that vtime reported by QMP matches the VTA expected time.
    """
    dtb, kernel = build_clock_suspend_artifacts()
    # Use slaved-icount to ensure QMP 'query-replay' returns meaningful icount/vtime
    extra_args = ["-device", "virtmcu-clock,node=0,mode=slaved-icount"]

    simulation.add_node(node_id=0, dtb=dtb, kernel=kernel, extra_args=extra_args)
    async with simulation as sim:
        for _ in range(5):
            await sim.vta.step(1_000_000)
            expected_ns = sim.vta.current_vtimes[0]

            # Query vtime via QMP
            assert sim.bridge is not None
            qmp_vtime = await sim.bridge.get_virtual_time_ns()

            # They should be very close
            assert abs(qmp_vtime - expected_ns) < 1000
