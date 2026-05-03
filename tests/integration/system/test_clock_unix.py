"""
SOTA Test Module: test_clock_unix

Context:
This module implements tests for the test_clock_unix subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_clock_unix.
"""

from __future__ import annotations

import shutil
import subprocess
from pathlib import Path
from typing import TYPE_CHECKING

import pytest

if TYPE_CHECKING:
    from tools.testing.virtmcu_test_suite.simulation import Simulation


def build_artifacts() -> tuple[Path, Path]:
    from tools.testing.env import WORKSPACE_ROOT

    workspace_root = WORKSPACE_ROOT
    dtb_path = workspace_root / "tests/fixtures/guest_apps/boot_arm/minimal.dtb"
    kernel_path = workspace_root / "tests/fixtures/guest_apps/boot_arm/hello.elf"

    if not dtb_path.exists() or not kernel_path.exists():
        subprocess.run([shutil.which("make") or "make", "-C", "tests/fixtures/guest_apps/boot_arm", "all"], check=True)

    return dtb_path, kernel_path


@pytest.mark.asyncio
async def test_clock_unix_socket(simulation: Simulation) -> None:
    """
    Verify clock with unix socket transport.
    """
    dtb_path, kernel_path = build_artifacts()

    from tools.testing.virtmcu_test_suite.transport import UnixTransportImpl

    transport = UnixTransportImpl()
    await transport.start()
    simulation.transport = transport

    extra_args = ["-device", f"virtmcu-clock,node=0,mode=slaved-unix,router={transport.clock_sock}"]
    simulation.add_node(node_id=0, dtb=dtb_path, kernel=kernel_path, extra_args=extra_args)

    try:
        async with simulation as sim:
            vta = sim.vta
            assert vta is not None
            # Initial sync (vtime should be 0 or close to it) is handled by sim.__aenter__

            # 2. Advance 1ms
            vtimes = await vta.step(1_000_000)
            assert vtimes[0] >= 1_000_000

            # 3. Advance 10ms
            vtimes = await vta.step(10_000_000)
            assert vtimes[0] >= 11_000_000

    finally:
        await transport.stop()
