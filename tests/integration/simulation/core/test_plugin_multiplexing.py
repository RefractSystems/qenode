"""
Regression test: Cross-DSO CPU Hook Multiplexing

Verifies that multiple dynamic plugins (DSOs) can independently register
CPU halt hooks without overwriting each other due to static data boundaries.

Historically, plugins like `virtmcu-clock` and `telemetry` each maintained their
own copy of the multiplexer array in Rust due to being compiled into separate
`.so` libraries. When both were loaded, the last one to initialize overwrote
the global C pointer (`virtmcu_cpu_halt_hook`), starving the other of events.
"""

from __future__ import annotations

import logging
from collections.abc import Callable
from pathlib import Path

import pytest

from tools.testing.virtmcu_test_suite.simulation import Simulation
from tools.testing.virtmcu_test_suite.topics import SimTopic

logger = logging.getLogger(__name__)


@pytest.fixture(scope="module")
def wfi_artifacts(guest_app_factory: Callable[[str], Path]) -> tuple[Path, Path]:
    app_dir = guest_app_factory("telemetry_wfi")
    dtb = app_dir / "test_telemetry.dtb"
    kernel = app_dir / "test_wfi.elf"
    return dtb, kernel


@pytest.mark.asyncio
async def test_halt_hook_multiplexing(simulation: Simulation, wfi_artifacts: tuple[Path, Path]) -> None:
    """
    Test that when both `telemetry` and `clock` (injected automatically by simulation)
    are loaded, they both receive halt hooks.

    If multiplexing fails:
    - If `clock` overwrites `telemetry`: We capture 0 telemetry events.
    - If `telemetry` overwrites `clock`: Virtual time stalls, and sim.run_until times out.
    """
    dtb, kernel = wfi_artifacts
    captured = []

    assert simulation.transport is not None

    def on_telemetry(payload: bytes) -> None:
        captured.append(payload)

    await simulation.transport.subscribe(SimTopic.telemetry_trace(0), on_telemetry)

    # Note: `virtmcu-clock` is automatically appended to extra_args by the Simulation framework.
    simulation.add_node(
        node_id=0,
        dtb=dtb,
        kernel=kernel,
        extra_args=[
            "-device",
            "virtmcu-transport-hub,id=hub0",
            "-device",
            "telemetry,transport=hub0",
        ],
    )

    async with simulation as sim:
        # Advance virtual time.
        # If the clock plugin didn't get its hook, vta.step() will hang/timeout.
        # If the telemetry plugin didn't get its hook, captured will remain empty.
        await sim.run_until(lambda: len(captured) > 0, timeout_ns=100_000_000, step_ns=10_000_000, timeout=10.0)

        assert len(captured) > 0, "Telemetry plugin did not receive CPU halt events!"

        # We successfully advanced time AND received telemetry. Multiplexing works.
