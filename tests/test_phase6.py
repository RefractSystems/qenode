import logging
import os
from pathlib import Path

import pytest

from tools.testing.virtmcu_test_suite.process import AsyncManagedProcess

logger = logging.getLogger(__name__)


@pytest.mark.asyncio
async def test_phase6_coordinator(zenoh_router, zenoh_coordinator, zenoh_session):  # noqa: ARG001
    """
    Phase 6 smoke test: Zenoh Multi-Node Coordinator.
    Migrated from test/phase6/smoke_test.sh
    """
    workspace_root = Path(Path(Path(__file__).parent.resolve().parent))

    env = os.environ.copy()
    env["ZENOH_ROUTER"] = zenoh_router
    env["PYTHONPATH"] = (
        str(Path(workspace_root) / "tools")
        + ":"
        + str(Path(workspace_root) / "test" / "phase6")
        + ":"
        + env.get("PYTHONPATH", "")
    )

    # 1. Run comprehensive test suite
    logger.info("Running complete_test.py...")
    async with AsyncManagedProcess(
        "python3",
        (Path(workspace_root) / "test/phase6/complete_test.py"),
        env=env,
    ) as proc:
        await proc.wait()
        assert proc.returncode == 0, f"complete_test.py failed:\nSTDOUT: {proc.stdout_text}\nSTDERR: {proc.stderr_text}"

    # 2. Run malformed packet survival test
    logger.info("Running repro_crash.py...")
    async with AsyncManagedProcess(
        "python3",
        (Path(workspace_root) / "test/phase6/repro_crash.py"),
        env=env,
    ) as proc:
        await proc.wait()
        assert proc.returncode == 0, f"repro_crash.py failed: {proc.stderr_text}"

    # 3. Run stress test
    # Note: stress test might be slow
    logger.info("Running stress_test.py...")
    async with AsyncManagedProcess(
        "python3",
        (Path(workspace_root) / "test/phase6/stress_test.py"),
        env=env,
    ) as proc:
        await proc.wait()
        assert proc.returncode == 0, f"stress_test.py failed: {proc.stderr_text}"

    # Check if coordinator is still alive
    assert zenoh_coordinator.returncode is None
