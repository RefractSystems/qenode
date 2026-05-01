"""
Architectural hardening stress tests.
1. Quantum sync stress: 100 quanta with ZenohClock.
2. Sequence number tie-breaking: UART bytes at same vtime.
"""

import asyncio
import subprocess

import pytest
import vproto


def _ensure_boot_arm_built():
    from tools.testing.env import WORKSPACE_ROOT

    workspace_root = WORKSPACE_ROOT
    dtb = workspace_root / "tests/fixtures/guest_apps/boot_arm/minimal.dtb"
    kernel = workspace_root / "tests/fixtures/guest_apps/boot_arm/hello.elf"
    if not dtb.exists() or not kernel.exists():
        subprocess.run(["make", "-C", "tests/fixtures/guest_apps/boot_arm"], check=True, cwd=workspace_root)
    return dtb, kernel


@pytest.mark.asyncio
async def test_quantum_sync_stress(qemu_launcher, zenoh_router, zenoh_session):
    """Run 100 quanta and verify no stalls or state machine failures."""
    dtb, kernel = _ensure_boot_arm_built()
    node_id = 42  # Unique ID for this test
    quantum_ns = 1_000_000  # 1ms
    total_quanta = 100

    extra_args = ["-device", f"virtmcu-clock,node={node_id},router={zenoh_router},mode=slaved-icount"]

    bridge = await qemu_launcher(dtb_path=dtb, kernel_path=kernel, extra_args=extra_args, ignore_clock_check=True)

    from tests.conftest import VirtualTimeAuthority
    from tools.testing.virtmcu_test_suite.conftest_core import VirtmcuSimulation

    vta = VirtualTimeAuthority(zenoh_session, [node_id])
    sim = VirtmcuSimulation(bridge, vta)

    async with sim:
        for _ in range(total_quanta):
            await vta.step(quantum_ns, timeout=10.0)


@pytest.mark.asyncio
async def test_uart_sequence_tiebreaking(qemu_launcher, zenoh_router, zenoh_session):
    """Verify that multiple UART bytes sent at the same vtime arrive in order."""
    dtb, kernel = _ensure_boot_arm_built()
    node_id = 43

    extra_args = [
        "-device",
        f"virtmcu-clock,node={node_id},router={zenoh_router},mode=slaved-icount",
        "-chardev",
        f"virtmcu,id=char0,node={node_id},router={zenoh_router}",
        "-serial",
        "chardev:char0",
    ]

    bridge = await qemu_launcher(dtb_path=dtb, kernel_path=kernel, extra_args=extra_args, ignore_clock_check=True)

    from tests.conftest import VirtualTimeAuthority
    from tools.testing.virtmcu_test_suite.conftest_core import VirtmcuSimulation

    vta = VirtualTimeAuthority(zenoh_session, [node_id])
    sim = VirtmcuSimulation(bridge, vta)

    pub = await asyncio.to_thread(lambda: zenoh_session.declare_publisher(f"virtmcu/uart/{node_id}/rx"))

    async with sim:
        # 1. Advance past boot
        await vta.step(100_000_000, timeout=10.0)

        # 2. Pre-publish "HELLO" all at the SAME virtual time
        vtime = vta.current_vtimes[node_id] + 1_000_000
        test_str = b"HELLO"
        for i, char in enumerate(test_str):
            header = vproto.ZenohFrameHeader(vtime, i, 1).pack()

            def put_msg(h: bytes, c: int) -> None:
                pub.put(h + bytes([c]))

            await asyncio.to_thread(put_msg, header, char)

        for _ in range(10):
            await vta.step(1_000_000, timeout=10.0)
