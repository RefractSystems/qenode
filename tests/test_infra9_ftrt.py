import asyncio
import logging
import os

import pytest

logger = logging.getLogger(__name__)


@pytest.mark.asyncio
async def test_infra9_faster_than_real_time(zenoh_router, qemu_launcher, zenoh_session):
    """
    INFRA-9: Proves that the simulation runs Faster-Than-Real-Time (FTRT)
    when pacing is disabled (multiplier = 0.0), unbound by pseudo-polling bottlenecks.
    """
    import subprocess
    from pathlib import Path

    workspace_root = Path(__file__).resolve().parent.parent
    kernel_path = workspace_root / "test/phase1/hello.elf"
    dtb_path = workspace_root / "test/phase1/minimal.dtb"
    if not kernel_path.exists():
        subprocess.run(["make", "-C", "test/phase1"], check=True, cwd=workspace_root)

    # Launch node in slaved-icount mode so we strictly govern its virtual clock
    extra_args = [
        "-device",
        f"virtmcu-clock,node=1,mode=slaved-icount,router={zenoh_router}",
        "-S",  # Start paused
    ]

    # Do NOT run this test under ASan/TSan or Miri where FTRT is impossible
    if os.environ.get("VIRTMCU_USE_ASAN") == "1" or os.environ.get("VIRTMCU_USE_TSAN") == "1":
        pytest.skip("ASan/TSan overhead inherently prevents Faster-Than-Real-Time execution.")

    bridge = await qemu_launcher(dtb_path, kernel_path, extra_args=extra_args, ignore_clock_check=True)

    from tests.conftest import VirtualTimeAuthority

    vta = VirtualTimeAuthority(zenoh_session, [1])

    await bridge.start_emulation()

    # Baseline clock synchronization
    await vta.step(0)

    loop = asyncio.get_running_loop()
    start_wall = loop.time()

    # Step exactly 20 seconds of virtual time
    target_virtual_ns = 20_000_000_000

    # Chunk the execution into 100ms blocks
    chunk_ns = 100_000_000
    for _ in range(target_virtual_ns // chunk_ns):
        await vta.step(chunk_ns, timeout=10.0)

    end_wall = loop.time()
    elapsed_wall = end_wall - start_wall

    logger.info(f"Executed 20.0s of Virtual Time in {elapsed_wall:.2f}s of Wall-Clock Time.")

    # Assert FTRT efficiency: 20s of virtual time MUST complete in < 5 seconds of real time.
    assert elapsed_wall < 5.0, (
        f"FTRT failed! Took {elapsed_wall}s to simulate 20s. Framework is likely bottlenecking execution."
    )
