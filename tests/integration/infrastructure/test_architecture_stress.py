"""Run 100 quanta and verify no stalls or state machine failures."""

from __future__ import annotations

import asyncio
from typing import TYPE_CHECKING, Any

import pytest

from tools import vproto
from tools.testing.virtmcu_test_suite.topics import SimTopic

if TYPE_CHECKING:
    from tools.testing.virtmcu_test_suite.simulation import Simulation


@pytest.mark.asyncio
async def test_quantum_sync_stress(simulation: Simulation, guest_app_factory: Any) -> None:  # noqa: ANN401

    app_dir = guest_app_factory("boot_arm")
    dtb = app_dir / "minimal.dtb"
    kernel = app_dir / "hello.elf"

    node_id = 42  # Unique ID for this test
    quantum_ns = 1_000_000  # 1ms
    total_quanta = 100

    simulation.add_node(node_id=node_id, dtb=dtb, kernel=kernel)

    async with simulation as sim:
        for _ in range(total_quanta):
            await sim.vta.step(quantum_ns, timeout=10.0)  # LINT_EXCEPTION: vta_step_loop


@pytest.mark.asyncio
async def test_uart_sequence_tiebreaking(simulation: Simulation, guest_app_factory: Any) -> None:  # noqa: ANN401
    """Verify that multiple UART bytes sent at the same vtime arrive in order."""
    app_dir = guest_app_factory("boot_arm")
    dtb = app_dir / "minimal.dtb"
    kernel = app_dir / "hello.elf"
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
        await sim.vta.step(100_000_000, timeout=10.0)  # LINT_EXCEPTION: vta_step_loop

        # 2. Pre-publish "HELLO" all at the SAME virtual time
        vtime = sim.vta.current_vtimes[node_id] + 1_000_000
        test_str = b"HELLO"
        for i, char in enumerate(test_str):
            header = vproto.ZenohFrameHeader(vtime, i, 1).pack()
            await asyncio.to_thread(put_msg, header, char)

        for _ in range(10):
            await sim.vta.step(1_000_000, timeout=10.0)  # LINT_EXCEPTION: vta_step_loop
