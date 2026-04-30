import asyncio
from unittest.mock import AsyncMock, patch

import pytest

from tests.conftest import VirtualTimeAuthority


def build_phase1_artifacts():
    import subprocess
    from pathlib import Path

    workspace_root = Path(__file__).resolve().parent.parent
    dtb_path = workspace_root / "tests/fixtures/guest_apps/phase1/minimal.dtb"
    kernel_path = workspace_root / "tests/fixtures/guest_apps/phase1/hello.elf"
    if not dtb_path.exists() or not kernel_path.exists():
        subprocess.run(["make", "-C", "tests/fixtures/guest_apps/phase1", "all"], check=True)
    return dtb_path, kernel_path


@pytest.mark.asyncio
async def test_watchdog_fires_on_vtime_stall(zenoh_router, qemu_launcher, zenoh_session):
    dtb_path, kernel_path = build_phase1_artifacts()
    extra_args = [
        "-S",
        "-device",
        f"virtmcu-clock,node=0,mode=slaved-suspend,router={zenoh_router}",
    ]
    bridge = await qemu_launcher(dtb_path, kernel_path, extra_args=extra_args, ignore_clock_check=True)
    vta = VirtualTimeAuthority(zenoh_session, [0])

    await bridge.start_emulation()
    await vta.step(0)

    # Mock the get_virtual_time_ns so it appears to be stalled at 1_000_000
    with patch.object(bridge, "get_virtual_time_ns", new_callable=AsyncMock, return_value=1_000_000):
        try:
            await asyncio.sleep(15.0)  # SLEEP_EXCEPTION: waiting for watchdog
        except asyncio.CancelledError as e:
            assert "Guest OS deadlocked" in str(e)
            return

    pytest.fail("Watchdog did not fire!")
