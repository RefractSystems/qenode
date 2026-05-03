"""
SOTA Test Module: test_stress

Context:
This module implements tests for the test_stress subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_stress.
"""

from __future__ import annotations

import asyncio
import os
from typing import TYPE_CHECKING

import pytest

from tools.testing.utils import get_time_multiplier

if TYPE_CHECKING:
    from pathlib import Path

    from tools.testing.virtmcu_test_suite.simulation import Simulation


_base_stall_timeout_ms = int(os.environ.get("VIRTMCU_STALL_TIMEOUT_MS", "5000"))
_STALL_TIMEOUT_MS = int(_base_stall_timeout_ms * get_time_multiplier())
_VTA_TIMEOUT_S: float = max(30.0, _STALL_TIMEOUT_MS / 1000.0 + 10.0)


@pytest.mark.parametrize("deterministic_coordinator", [{"nodes": 3, "pdes": True}], indirect=True)
@pytest.mark.asyncio
async def test_stress(
    deterministic_coordinator: asyncio.subprocess.Process,
    simulation: Simulation,
    tmp_path: Path,
) -> None:
    """
    Stress tests the TA/Coordinator Synchronization Protocol using the REAL deterministic_coordinator.
    Runs for 50 quanta to ensure the barrier logic does not deadlock or drop signals under load.
    """
    import logging

    logging.getLogger(__name__)

    from tools.testing.env import WORKSPACE_ROOT

    workspace_root = WORKSPACE_ROOT
    firmware_path = workspace_root / "tests/fixtures/guest_apps/uart_echo/echo.elf"
    if not firmware_path.exists():
        pytest.fail("echo.elf not found — run 'make -C tests/fixtures/guest_apps/uart_echo' first")

    board_yaml = tmp_path / "board.yaml"
    board_yaml.write_text(
        """
machine:
  cpus:
    - name: cpu0
      type: cortex-a15
memory:
  - name: sram
    address: 0x40000000
    size: 0x1000000
peripherals:
  - name: uart0
    type: pl011
    address: 0x09000000
    interrupt: 4
"""
    )

    for i in range(3):
        args = [
            "virtmcu-clock,coordinated=true",
            "-chardev",
            f"virtmcu,id=chr{i},topic=sim/uart",
            "-serial",
            f"chardev:chr{i}",
        ]
        simulation.add_node(
            node_id=i,
            dtb=board_yaml,
            kernel=firmware_path,
            extra_args=args,
        )
        async with simulation as sim:
            # Run 50 quanta
            for _i in range(50):
                await sim.vta.step(delta_ns=1_000_000, timeout=_VTA_TIMEOUT_S)

