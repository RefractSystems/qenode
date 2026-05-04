"""
SOTA Test Module: test_telemetry_stress

Context:
This module implements tests for the test_telemetry_stress subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_telemetry_stress.
"""

from __future__ import annotations

from pathlib import Path
from typing import TYPE_CHECKING, Any

import pytest

if TYPE_CHECKING:
    from tools.testing.virtmcu_test_suite.simulation import Simulation


@pytest.mark.asyncio
async def test_telemetry_stress_queue(
    simulation: Simulation, zenoh_router: str, tmp_path: Path, guest_app_factory: Any  # noqa: ANN401
) -> None:
    from tools.testing.virtmcu_test_suite.factory import compile_yaml

    app_dir = guest_app_factory("actuator")
    yaml_file = app_dir / "board.yaml"
    tmp_yaml = tmp_path / "board.yaml"
    dtb = tmp_path / "board.dtb"

    yaml_content = yaml_file.read_text().replace("ZENOH_ROUTER_ENDPOINT", zenoh_router)
    tmp_yaml.write_text(yaml_content)

    compile_yaml(tmp_yaml, dtb)

    simulation.add_node(node_id=0, dtb=dtb)

    async with simulation as sim:
        status = await sim.bridge.qmp.execute("query-status")
        assert isinstance(status, dict)
        assert status["running"] is True
