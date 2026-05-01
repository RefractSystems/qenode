"""
SOTA Test Module: test_coordinator

Context:
This module implements tests for the test_coordinator subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_coordinator.
"""

import logging
import os
import subprocess
from pathlib import Path

import pytest

logger = logging.getLogger(__name__)


@pytest.mark.asyncio
async def test_coordinator(zenoh_router, zenoh_coordinator, zenoh_session):  # noqa: ARG001
    """
    smoke test: Zenoh Multi-Node Coordinator.
    Migrated from tests/fixtures/guest_apps/coordinator_stress/smoke_test.sh
    """
    from tools.testing.env import WORKSPACE_ROOT

    workspace_root = WORKSPACE_ROOT

    env = os.environ.copy()
    env["ZENOH_ROUTER"] = zenoh_router
    env["PYTHONPATH"] = (
        str(Path(workspace_root) / "tools")
        + ":"
        + str(Path(workspace_root) / "tests" / "fixtures" / "guest_apps" / "coordinator_stress")
        + ":"
        + env.get("PYTHONPATH", "")
    )

    # 1. Run comprehensive test suite
    logger.info("Running complete_test.py...")
    ret = subprocess.run(
        ["python3", "-u", str(Path(workspace_root) / "tests/fixtures/guest_apps/coordinator_stress/complete_test.py")],
        env=env,
        check=False,
    )
    assert ret.returncode == 0, "complete_test.py failed"

    # 2. Run malformed packet survival test
    logger.info("Running repro_crash.py...")
    ret = subprocess.run(
        ["python3", "-u", str(Path(workspace_root) / "tests/fixtures/guest_apps/coordinator_stress/repro_crash.py")],
        env=env,
        check=False,
    )
    assert ret.returncode == 0, "repro_crash.py failed"

    # 3. Run stress test
    logger.info("Running stress_test.py...")
    ret = subprocess.run(
        ["python3", "-u", str(Path(workspace_root) / "tests/fixtures/guest_apps/coordinator_stress/stress_test.py")],
        env=env,
        check=False,
    )
    assert ret.returncode == 0, "stress_test.py failed"

    # Check if coordinator is still alive
    assert zenoh_coordinator.returncode is None
