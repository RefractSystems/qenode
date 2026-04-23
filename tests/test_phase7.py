import asyncio
import struct
import subprocess
from functools import partial
from pathlib import Path

import pytest


def build_phase7_artifacts():
    workspace_root = Path(__file__).resolve().parent.parent
    dtb_path = workspace_root / "test/phase1/minimal.dtb"
    kernel_path = workspace_root / "test/phase1/hello.elf"

    if not dtb_path.exists():
        subprocess.run(["make", "-C", "test/phase1", "minimal.dtb"], check=True)

    return dtb_path, kernel_path


@pytest.mark.asyncio
async def test_phase7_clock_suspend(zenoh_router, qemu_launcher, zenoh_session):
    """
    Phase 7: zenoh-clock in slaved-suspend mode.
    Verify that virtual time advances and matches queries.
    """
    dtb_path, kernel_path = build_phase7_artifacts()

    extra_args = ["-device", f"zenoh-clock,node=0,mode=slaved-suspend,router={zenoh_router}"]

    await qemu_launcher(dtb_path, kernel_path, extra_args=extra_args, ignore_clock_check=True)

    from tests.conftest import VirtualTimeAuthority
    vta = VirtualTimeAuthority(zenoh_session, [0])

    try:
        # 1. Initial sync should succeed
        await vta.step(0)
        vtime = (await vta.step(1_000_000))[0]
        assert vtime >= 1_000_000

        # 2. Advance clock significantly
        vtime = (await vta.step(100_000_000))[0]
        assert vtime >= 101_000_000

    finally:
        pass


@pytest.mark.asyncio
async def test_phase7_clock_stall(zenoh_router, qemu_launcher, zenoh_session):
    """
    Phase 7: zenoh-clock stall detection.
    """
    dtb_path, kernel_path = build_phase7_artifacts()

    extra_args = ["-device", f"zenoh-clock,node=0,mode=slaved-suspend,router={zenoh_router},stall-timeout=500"]

    await qemu_launcher(dtb_path, kernel_path, extra_args=extra_args, ignore_clock_check=True)

    from tests.conftest import VirtualTimeAuthority
    vta = VirtualTimeAuthority(zenoh_session, [0])

    await vta.step(0)

    import psutil
    qemu_proc = None
    for p in psutil.process_iter(["cmdline"]):
        if "qemu-system-arm" in (p.info["cmdline"] or []) and str(dtb_path) in str(p.info["cmdline"]):
            qemu_proc = p
            break

    assert qemu_proc is not None, "Could not find QEMU process"
    qemu_proc.suspend()

    try:
        with pytest.raises(RuntimeError, match="reported CLOCK STALL"):
            await vta.step(10_000_000, timeout=5.0)

        qemu_proc.resume()
        vtime = (await vta.step(1_000_000))[0]
        assert vtime > 0

    finally:
        qemu_proc.resume()


@pytest.mark.asyncio
async def test_phase7_netdev(zenoh_router, qemu_launcher, zenoh_session):
    """
    Phase 7: zenoh-netdev basic packet delivery.
    """
    dtb_path, kernel_path = build_phase7_artifacts()

    extra_args = [
        "-icount",
        "shift=0,align=off,sleep=off",
        "-device",
        f"zenoh-clock,node=1,mode=slaved-icount,router={zenoh_router}",
        "-netdev",
        f"zenoh,node=1,id=n1,router={zenoh_router}",
    ]
    await qemu_launcher(dtb_path, kernel_path, extra_args=extra_args, ignore_clock_check=True)

    from tests.conftest import VirtualTimeAuthority
    vta = VirtualTimeAuthority(zenoh_session, [1])

    await vta.step(0)

    NETDEV_TOPIC = "sim/eth/frame/1/rx"  # noqa: N806
    DELIVERY_VTIME_NS = 500_000  # noqa: N806
    FRAME = b"\xff" * 14  # noqa: N806
    packet = struct.pack("<QI", DELIVERY_VTIME_NS, len(FRAME)) + FRAME

    pub = await asyncio.to_thread(lambda: zenoh_session.declare_publisher(NETDEV_TOPIC))

    from tests.conftest import wait_for_zenoh_discovery
    await wait_for_zenoh_discovery(zenoh_session, NETDEV_TOPIC)

    await asyncio.to_thread(lambda: pub.put(packet))

    await vta.step(1_000_000)
    assert vta.current_vtimes[1] == 1_000_000


@pytest.mark.asyncio
async def test_phase7_netdev_stress(zenoh_router, qemu_launcher, zenoh_session):
    """
    Phase 7: zenoh-netdev stress test.
    """
    dtb_path, kernel_path = build_phase7_artifacts()

    extra_args = [
        "-icount",
        "shift=0,align=off,sleep=off",
        "-device",
        f"zenoh-clock,node=0,mode=slaved-icount,router={zenoh_router}",
        "-netdev",
        f"zenoh,node=0,id=n0,router={zenoh_router}",
    ]
    await qemu_launcher(dtb_path, kernel_path, extra_args=extra_args, ignore_clock_check=True)

    from tests.conftest import VirtualTimeAuthority
    vta = VirtualTimeAuthority(zenoh_session, [0])

    await vta.step(0)

    for i in range(100, 0, -1):
        vtime = i * 1_000_000
        packet = struct.pack("<QI", vtime, 14) + b"\xee" * 14
        await asyncio.to_thread(partial(zenoh_session.put, "sim/eth/frame/0/rx", packet))

    await vta.step(200_000_000)
    assert vta.current_vtimes[0] >= 200_000_000


@pytest.mark.asyncio
async def test_phase7_determinism(zenoh_router, qemu_launcher, zenoh_session):
    """
    Phase 7: Clock/Netdev determinism.
    """
    dtb_path, kernel_path = build_phase7_artifacts()

    extra_args = [
        "-icount",
        "shift=0,align=off,sleep=off",
        "-device",
        f"zenoh-clock,node=0,mode=slaved-icount,router={zenoh_router}",
        "-netdev",
        f"zenoh,node=0,id=n0,router={zenoh_router}",
    ]
    await qemu_launcher(dtb_path, kernel_path, extra_args=extra_args, ignore_clock_check=True)

    from tests.conftest import VirtualTimeAuthority
    vta = VirtualTimeAuthority(zenoh_session, [0])

    await vta.step(0)

    packet = struct.pack("<QI", 5_000_000, 14) + b"\xdd" * 14
    await asyncio.to_thread(lambda: zenoh_session.put("sim/eth/frame/0/rx", packet))

    await vta.step(10_000_000)
    assert vta.current_vtimes[0] == 10_000_000
