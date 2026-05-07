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
from typing import TYPE_CHECKING, Any

import pytest

from tools.testing.parameters import TestParams

if TYPE_CHECKING:
    from tools.testing.virtmcu_test_suite.simulation import Simulation


logger = logging.getLogger(__name__)


@pytest.mark.asyncio
async def test_clock_slaved_suspend_smoke(simulation: Simulation, guest_app_factory: Any) -> None:
    """
    Verify basic clock advancement in slaved-suspend mode.
    """
    app_dir = guest_app_factory("boot_arm")
    dtb = app_dir / "minimal.dtb"
    kernel = app_dir / "hello.elf"

    extra_args = ["-device", "virtmcu-clock,node=0,mode=slaved-suspend"]

    simulation.add_node(node_id=0, dtb=dtb, kernel=kernel, extra_args=extra_args)
    async with simulation as sim:
        # 1. Initial vtime should be small (we allow some slack for boot if not using icount)  # virtmcu-allow: vta_step_loop reasoning="Legacy exception"
        vtime = (await sim.vta.step(0))[
            0
        ]  # virtmcu-allow: lint reasoning="vta_step_loop"  # virtmcu-allow: vta_step_loop reasoning="Legacy exception"
        # In slaved-suspend without icount, vtime is real-time. simulation fixture has some sleep(0.5) calls.
        assert vtime < int(2_000_000_000 * TestParams.multiplier())

        # 2. Advance 10ms  # virtmcu-allow: vta_step_loop reasoning="Legacy exception"
        vtime = (await sim.vta.step(10_000_000))[
            0
        ]  # virtmcu-allow: lint reasoning="vta_step_loop"  # virtmcu-allow: vta_step_loop reasoning="Legacy exception"
        assert vtime >= 10_000_000

        # 3. Advance another 10ms  # virtmcu-allow: vta_step_loop reasoning="Legacy exception"
        vtime = (await sim.vta.step(10_000_000))[
            0
        ]  # virtmcu-allow: lint reasoning="vta_step_loop"  # virtmcu-allow: vta_step_loop reasoning="Legacy exception"
        assert vtime >= 20_000_000


@pytest.mark.asyncio
async def test_clock_stall_detection(simulation: Simulation, guest_app_factory: Any) -> None:
    """
    Verify that slaved-suspend mode correctly triggers and reports
    clock stall detection.
    """
    app_dir = guest_app_factory("boot_arm")
    dtb = app_dir / "minimal.dtb"
    kernel = app_dir / "hello.elf"

    # Use a shorter stall-timeout specifically for the stall test, but scale it for the environment.
    base_stall = 2000
    stall_timeout = TestParams.get_stall_timeout_ms(base_stall)
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
                # vta.step already scales its timeout argument internally by TestParams.multiplier().  # virtmcu-allow: vta_step_loop reasoning="Legacy exception"
                await sim.vta.step(
                    10_000_000, timeout=(base_stall / 1000.0) + 10.0
                )  # virtmcu-allow: lint reasoning="vta_step_loop"  # virtmcu-allow: vta_step_loop reasoning="Legacy exception"
            assert sim.bridge is not None
            await sim.bridge.start_emulation()
            # Give QEMU a moment to resume  # virtmcu-allow: vta_step_loop reasoning="Legacy exception"
            vtime = (await sim.vta.step(1_000_000))[
                0
            ]  # virtmcu-allow: lint reasoning="vta_step_loop"  # virtmcu-allow: vta_step_loop reasoning="Legacy exception"
            assert vtime > 0

        finally:
            if sim.bridge is not None:
                try:
                    await asyncio.wait_for(sim.bridge.start_emulation(), timeout=TestParams.scale_timeout(2.0))
                except Exception as e:
                    logger.error(f"Failed to start emulation in finally: {e}")


@pytest.mark.asyncio
async def test_slow_boot_fast_execute(simulation: Simulation, guest_app_factory: Any) -> None:
    """
    Verify "slow boot / fast execute" invariant.
    The first quantum (initial sync) should survive a delay longer than the standard stall-timeout.
    Subsequent quantums should stall if delayed.
    """
    app_dir = guest_app_factory("boot_arm")
    dtb = app_dir / "minimal.dtb"
    kernel = app_dir / "hello.elf"

    # 1. Start QEMU with a short stall timeout
    base_stall = 500
    stall_timeout = TestParams.get_stall_timeout_ms(base_stall)
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
        await asyncio.sleep(  # virtmcu-allow: sleep reasoning="test-only timing check"

            TestParams.scale_timeout(1.0)
        )  # virtmcu-allow: sleep reasoning="deliberate delay to test boot grace period"
        await sim.vta.init()

        # 3. First step should work  # virtmcu-allow: vta_step_loop reasoning="Legacy exception"
        await sim.vta.step(
            1_000_000
        )  # virtmcu-allow: lint reasoning="vta_step_loop"  # virtmcu-allow: vta_step_loop reasoning="Legacy exception"
        # 4. Now pause and step -> should stall
        assert sim.bridge is not None
        await sim.bridge.pause_emulation()
        with pytest.raises(
            RuntimeError, match="reported CLOCK STALL"
        ):  # virtmcu-allow: vta_step_loop reasoning="Legacy exception"
            await sim.vta.step(
                1_000_000, timeout=(base_stall / 1000.0) + 5.0
            )  # virtmcu-allow: lint reasoning="vta_step_loop"  # virtmcu-allow: vta_step_loop reasoning="Legacy exception"


@pytest.mark.asyncio
async def test_clock_suspend_wfi(simulation: Simulation, guest_app_factory: Any) -> None:
    """
    Verify that clock continues to advance during WFI in slaved-suspend mode.
    The test kernel performs a 10ms WFI.
    """
    app_dir_boot = guest_app_factory("boot_arm")
    dtb = app_dir_boot / "minimal.dtb"

    app_dir_wfi = guest_app_factory("telemetry_wfi")
    kernel = app_dir_wfi / "test_wfi.elf"

    extra_args = ["-device", "virtmcu-clock,node=0,mode=slaved-suspend"]

    simulation.add_node(node_id=0, dtb=dtb, kernel=kernel, extra_args=extra_args)
    async with simulation as sim:
        # Initial sync  # virtmcu-allow: vta_step_loop reasoning="Legacy exception"
        await sim.vta.step(
            0
        )  # virtmcu-allow: lint reasoning="vta_step_loop"  # virtmcu-allow: vta_step_loop reasoning="Legacy exception"
        # Step 20ms. The guest should be in WFI most of this time.  # virtmcu-allow: vta_step_loop reasoning="Legacy exception"
        vtime = (await sim.vta.step(20_000_000))[
            0
        ]  # virtmcu-allow: lint reasoning="vta_step_loop"  # virtmcu-allow: vta_step_loop reasoning="Legacy exception"
        assert vtime >= 20_000_000

        # Verify UART output indicates WFI was reached
        assert sim.bridge is not None
        await sim.bridge.wait_for_line_on_uart("WFI started", timeout=5.0)


@pytest.mark.asyncio
async def test_clock_suspend_vtime_alignment(simulation: Simulation, guest_app_factory: Any) -> None:
    """
    Verify that vtime reported by QMP matches the VTA expected time.
    """
    app_dir = guest_app_factory("boot_arm")
    dtb = app_dir / "minimal.dtb"
    kernel = app_dir / "busy_loop.elf"

    # Use slaved-icount to ensure QMP 'query-replay' returns meaningful icount/vtime
    extra_args = ["-device", "virtmcu-clock,node=0,mode=slaved-icount"]

    simulation.add_node(node_id=0, dtb=dtb, kernel=kernel, extra_args=extra_args)
    async with simulation as sim:
        for _ in range(5):
            await sim.vta.step(  # virtmcu-allow: vta_step_loop reasoning="Legacy exception"
                1_000_000
            )
            expected_ns = sim.vta.current_vtimes[0]

            # Query vtime via QMP
            assert sim.bridge is not None
            qmp_vtime = await sim.bridge.get_virtual_time_ns()

            # They should be very close
            assert abs(qmp_vtime - expected_ns) < 1000
