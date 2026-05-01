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
from tools.testing.virtmcu_test_suite.conftest_core import VirtualTimeAuthority

if TYPE_CHECKING:
    from pathlib import Path

    import zenoh


_base_stall_timeout_ms = int(os.environ.get("VIRTMCU_STALL_TIMEOUT_MS", "5000"))
_STALL_TIMEOUT_MS = int(_base_stall_timeout_ms * get_time_multiplier())
_VTA_TIMEOUT_S: float = max(30.0, _STALL_TIMEOUT_MS / 1000.0 + 10.0)


@pytest.mark.parametrize("zenoh_coordinator", [{"nodes": 3, "pdes": True}], indirect=True)
@pytest.mark.asyncio
async def test_stress(
    zenoh_router: str,
    zenoh_session: zenoh.Session,
    zenoh_coordinator: asyncio.subprocess.Process,
    qemu_launcher: object,
    tmp_path: Path,
) -> None:
    """
    Stress tests the TA/Coordinator Synchronization Protocol using the REAL zenoh_coordinator.
    Runs for 50 quanta to ensure the barrier logic does not deadlock or drop signals under load.
    """
    import logging

    logger = logging.getLogger(__name__)

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

    icount_args = ["-icount", "shift=0,align=off,sleep=off"]

    nodes = []
    for i in range(3):
        args = [
            "-device",
            f"virtmcu-clock,node={i},mode=slaved-icount,router={zenoh_router},coordinated=true",
            "-chardev",
            f"virtmcu,id=chr{i},node={i},router={zenoh_router},topic=sim/uart",
            "-serial",
            f"chardev:chr{i}",
        ]
        n = await qemu_launcher(  # type: ignore[operator]
            str(board_yaml),
            firmware_path,
            ignore_clock_check=True,
            extra_args=["-S", *icount_args, *args],
        )
        nodes.append(n)

    vta = VirtualTimeAuthority(zenoh_session, node_ids=[0, 1, 2])

    async def _stream_output(stream: asyncio.StreamReader, name: str) -> None:
        while True:
            line = await stream.readline()
            if not line:
                break
            logger.info(f"Coordinator {name}: {line.decode().strip()}")

    _output_tasks = [
        asyncio.create_task(_stream_output(zenoh_coordinator.stdout, "STDOUT")),  # type: ignore[arg-type]
        asyncio.create_task(_stream_output(zenoh_coordinator.stderr, "STDERR")),  # type: ignore[arg-type]
    ]

    try:
        from tools.testing.virtmcu_test_suite.conftest_core import VirtmcuSimulation

        async with VirtmcuSimulation(nodes, vta):
            # Run 50 quanta
            for _i in range(50):
                await vta.step(delta_ns=1_000_000, timeout=_VTA_TIMEOUT_S)

    finally:
        for task in _output_tasks:
            task.cancel()
            import contextlib
            with contextlib.suppress(asyncio.CancelledError):
                await task
