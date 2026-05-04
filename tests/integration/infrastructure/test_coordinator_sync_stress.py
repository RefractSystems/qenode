from __future__ import annotations

from collections.abc import Callable
from typing import TYPE_CHECKING

import pytest

if TYPE_CHECKING:
    from pathlib import Path

    from tools.testing.virtmcu_test_suite.simulation import Simulation


@pytest.mark.parametrize("deterministic_coordinator", [{"nodes": 3, "pdes": True}], indirect=True)
@pytest.mark.asyncio
async def test_stress(
    simulation: Simulation,
    tmp_path: Path,
    guest_app_factory: Callable[[str], Path], deterministic_coordinator: object,
) -> None:
    """
    Stress tests the TA/Coordinator Synchronization Protocol using the REAL deterministic_coordinator.
    Runs for 50 quanta to ensure the barrier logic does not deadlock or drop signals under load.
    """
    app_dir = guest_app_factory("telemetry_wfi")
    firmware_path = app_dir / "test_wfi.elf"

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
    interrupt: 33
    transport: zenoh
    node: 0
"""
    )

    simulation.add_node(node_id=0, dtb=board_yaml, kernel=firmware_path)

    async with simulation:
        vta = simulation.vta
        # Run for 50 quanta (10ms each)
        for _ in range(50):
            await vta.step(10_000_000)
            # Ensure we are actually advancing
            assert vta.current_vtimes[0] > 0
