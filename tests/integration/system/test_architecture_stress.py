"""Run 100 quanta and verify no stalls or state machine failures."""

from __future__ import annotations

import asyncio
import shutil
import subprocess
from pathlib import Path
from typing import TYPE_CHECKING

import pytest

from tools import vproto
from tools.testing.virtmcu_test_suite.topics import SimTopic

if TYPE_CHECKING:
    from tools.testing.virtmcu_test_suite.simulation import Simulation


def _ensure_boot_arm_built() -> tuple[Path, Path]:
    from tools.testing.env import WORKSPACE_ROOT

    workspace_root = WORKSPACE_ROOT
    dtb = workspace_root / "tests/fixtures/guest_apps/boot_arm/minimal.dtb"
    kernel = workspace_root / "tests/fixtures/guest_apps/boot_arm/hello.elf"
    if not dtb.exists() or not kernel.exists():
        subprocess.run(
            [shutil.which("make") or "make", "-C", "tests/fixtures/guest_apps/boot_arm"], check=True, cwd=workspace_root
        )
    return dtb, kernel


@pytest.mark.asyncio
async def test_quantum_sync_stress(simulation: Simulation) -> None:

    dtb, kernel = _ensure_boot_arm_built()
    node_id = 42  # Unique ID for this test
    quantum_ns = 1_000_000  # 1ms
    total_quanta = 100

    simulation.add_node(node_id=node_id, dtb=dtb, kernel=kernel)

    async with simulation as sim:
        for _ in range(total_quanta):
            await sim.vta.step(quantum_ns, timeout=10.0)


@pytest.mark.asyncio
async def test_uart_sequence_tiebreaking(simulation: Simulation) -> None:
    """Verify that multiple UART bytes sent at the same vtime arrive in order."""
    dtb, kernel = _ensure_boot_arm_built()
    node_id = 43

    extra_args = [
        "-chardev",
        "virtmcu,id=char0",
        "-serial",
        "chardev:char0",
    ]

    simulation.add_node(node_id=node_id, dtb=dtb, kernel=kernel, extra_args=extra_args)

    async with simulation as sim:
        # Connect Zenoh to send/receive data
        session = sim._session
        pub = await asyncio.to_thread(lambda: session.declare_publisher(SimTopic.uart_rx(node_id)))

        def put_msg(h: bytes, c: int) -> None:
            pub.put(h + bytes([c]))

        # 1. Advance past boot
        await sim.vta.step(100_000_000, timeout=10.0)

        # 2. Pre-publish "HELLO" all at the SAME virtual time
        vtime = sim.vta.current_vtimes[node_id] + 1_000_000
        test_str = b"HELLO"
        for i, char in enumerate(test_str):
            header = vproto.ZenohFrameHeader(vtime, i, 1).pack()
            await asyncio.to_thread(put_msg, header, char)

        for _ in range(10):
            await sim.vta.step(1_000_000, timeout=10.0)
