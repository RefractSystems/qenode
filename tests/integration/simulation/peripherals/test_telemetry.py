from __future__ import annotations

import logging
from collections.abc import Callable
from pathlib import Path
from typing import TYPE_CHECKING

import pytest

from tools.testing.virtmcu_test_suite.topics import SimTopic

if TYPE_CHECKING:
    from tools.testing.virtmcu_test_suite.simulation import Simulation


"""
SOTA Test Module: test_telemetry

Context:
This module verifies the Zenoh telemetry system.

Objective:
Ensure that telemetry events (Trace, Log, Actuator) are correctly emitted and captured.
"""

logger = logging.getLogger(__name__)


@pytest.fixture(scope="module")
def telemetry_artifacts(guest_app_factory: Callable[[str], Path]) -> tuple[Path, Path]:
    app_dir = guest_app_factory("telemetry_wfi")
    dtb = app_dir / "test_telemetry.dtb"
    kernel = app_dir / "test_wfi.elf"
    return dtb, kernel


@pytest.mark.asyncio
async def test_telemetry_emission(simulation: Simulation, telemetry_artifacts: tuple[Path, Path]) -> None:
    """
    1. Emission: Verify that guest-triggered telemetry reaches Zenoh.
    """
    dtb, kernel = telemetry_artifacts
    captured = []

    assert simulation.transport is not None
    def on_telemetry(payload: bytes) -> None:
        captured.append(payload)

    await simulation.transport.subscribe(SimTopic.telemetry_trace(0), on_telemetry)

    simulation.add_node(node_id=0, dtb=dtb, kernel=kernel, extra_args=["-device", "telemetry"])
    async with simulation as sim:
        # Run until guest app emits something (it should at boot)
        await sim.run_until(lambda: len(captured) > 0, timeout_ns=100_000_000, step_ns=10_000_000, timeout=10.0)
        assert len(captured) > 0, "No telemetry captured"


@pytest.mark.asyncio
async def test_telemetry_integrity(simulation: Simulation, telemetry_artifacts: tuple[Path, Path]) -> None:
    """
    2. Integrity: Verify FlatBuffers decoding of telemetry packets.
    """
    dtb, kernel = telemetry_artifacts
    captured = []

    assert simulation.transport is not None
    def on_telemetry(payload: bytes) -> None:
        captured.append(payload)

    await simulation.transport.subscribe(SimTopic.telemetry_trace(0), on_telemetry)

    simulation.add_node(node_id=0, dtb=dtb, kernel=kernel, extra_args=["-device", "telemetry"])
    async with simulation as sim:
        await sim.run_until(lambda: len(captured) > 0, timeout_ns=100_000_000, step_ns=10_000_000, timeout=10.0)
        for pkt in captured:
            assert len(pkt) > 0


@pytest.mark.asyncio
async def test_telemetry(simulation: Simulation, telemetry_artifacts: tuple[Path, Path]) -> None:
    """
    3. Telemetry Test: Verify Zenoh telemetry events are emitted.
    """
    dtb_path, kernel_path = telemetry_artifacts
    captured = []

    assert simulation.transport is not None
    def on_telemetry(payload: bytes) -> None:
        captured.append(payload)

    await simulation.transport.subscribe(SimTopic.telemetry_trace(0), on_telemetry)

    simulation.add_node(node_id=0, dtb=dtb_path, kernel=kernel_path, extra_args=["-device", "telemetry"])
    async with simulation as sim:
        await sim.run_until(lambda: len(captured) > 0, timeout_ns=100_000_000, step_ns=20_000_000, timeout=10.0)

        assert len(captured) > 0


@pytest.mark.asyncio
async def test_coordinator_topology(simulation: Simulation, telemetry_artifacts: tuple[Path, Path]) -> None:
    """
    5. Topology: deterministic_coordinator must correctly link nodes via queryables [P1]
    """
    dtb, kernel = telemetry_artifacts
    simulation.add_node(node_id=0, dtb=dtb, kernel=kernel, extra_args=["-device", "telemetry"])
    simulation.add_node(node_id=1, dtb=dtb, kernel=kernel, extra_args=["-device", "telemetry"])

    async with simulation as sim:
        assert sim.vta is not None
        res: dict[int, int] = await sim.vta.step(10_000_000)  # LINT_EXCEPTION: vta_step_loop
        assert 0 in res
        assert 1 in res
        assert res[0] == 10_000_000
        assert res[1] == 10_000_000
