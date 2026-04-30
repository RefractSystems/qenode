import logging
import os
import subprocess
from pathlib import Path

import pytest

logger = logging.getLogger(__name__)


@pytest.mark.asyncio
async def test_phase6_coordinator(zenoh_router, zenoh_coordinator, zenoh_session):  # noqa: ARG001
    """
    Phase 6 smoke test: Zenoh Multi-Node Coordinator.
    Migrated from tests/fixtures/guest_apps/phase6/smoke_test.sh
    """
    workspace_root = Path(Path(Path(__file__).parent.resolve().parent))

    env = os.environ.copy()
    env["ZENOH_ROUTER"] = zenoh_router
    env["PYTHONPATH"] = (
        str(Path(workspace_root) / "tools")
        + ":"
        + str(Path(workspace_root) / "tests" / "fixtures" / "guest_apps" / "phase6")
        + ":"
        + env.get("PYTHONPATH", "")
    )

    # 1. Run comprehensive test suite
    logger.info("Running complete_test.py...")
    ret = subprocess.run(
        ["python3", "-u", str(Path(workspace_root) / "tests/fixtures/guest_apps/phase6/complete_test.py")],
        env=env,
        check=False,
    )
    assert ret.returncode == 0, "complete_test.py failed"

    # 2. Run malformed packet survival test
    logger.info("Running repro_crash.py...")
    ret = subprocess.run(
        ["python3", "-u", str(Path(workspace_root) / "tests/fixtures/guest_apps/phase6/repro_crash.py")],
        env=env,
        check=False,
    )
    assert ret.returncode == 0, "repro_crash.py failed"

    # 3. Run stress test
    logger.info("Running stress_test.py...")
    ret = subprocess.run(
        ["python3", "-u", str(Path(workspace_root) / "tests/fixtures/guest_apps/phase6/stress_test.py")],
        env=env,
        check=False,
    )
    assert ret.returncode == 0, "stress_test.py failed"

    # Check if coordinator is still alive
    assert zenoh_coordinator.returncode is None
