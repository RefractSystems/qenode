"""
SOTA Test Module: test_coordinator

Context:
This module implements tests for the test_coordinator subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_coordinator.
"""

from __future__ import annotations

import asyncio.subprocess
import logging
import os
import shutil
import subprocess
from pathlib import Path
from typing import TYPE_CHECKING

import pytest

if TYPE_CHECKING:
    from pathlib import Path


logger = logging.getLogger(__name__)


@pytest.mark.asyncio
@pytest.mark.usefixtures("zenoh_session")
async def test_coordinator(zenoh_router: str, deterministic_coordinator: asyncio.subprocess.Process) -> None:
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
        [
            shutil.which("python3") or "python3",
            "-u",
            str(Path(workspace_root) / "tests/fixtures/guest_apps/coordinator_stress/complete_test.py"),
        ],
        env=env,
        check=False,
    )
    assert ret.returncode == 0, "complete_test.py failed"

    # 2. Run malformed packet survival test
    logger.info("Running repro_crash.py...")
    ret = subprocess.run(
        [
            shutil.which("python3") or "python3",
            "-u",
            str(Path(workspace_root) / "tests/fixtures/guest_apps/coordinator_stress/repro_crash.py"),
        ],
        env=env,
        check=False,
    )
    assert ret.returncode == 0, "repro_crash.py failed"

    # 3. Run stress test
    logger.info("Running stress_test.py...")
    ret = subprocess.run(
        [
            shutil.which("python3") or "python3",
            "-u",
            str(Path(workspace_root) / "tests/fixtures/guest_apps/coordinator_stress/stress_test.py"),
        ],
        env=env,
        check=False,
    )
    assert ret.returncode == 0, "stress_test.py failed"

    # Check if coordinator is still alive
    assert deterministic_coordinator.returncode is None
