# ZENOH_HACK_EXCEPTION: Tests zenoh_coordinator natively by mocking QEMU nodes
"""
SOTA Test Module: test_coordinator

Context:
This module implements tests for the test_coordinator subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_coordinator.
"""

from __future__ import annotations

import logging
import os
import sys
from pathlib import Path
from typing import TYPE_CHECKING

import pytest

if TYPE_CHECKING:
    from tools.testing.virtmcu_test_suite.conftest_core import ManagedSubprocess


logger = logging.getLogger(__name__)


@pytest.mark.asyncio
@pytest.mark.usefixtures("zenoh_session")
async def test_coordinator(zenoh_router: str, deterministic_coordinator: ManagedSubprocess) -> None:
    """
    smoke test: Zenoh Multi-Node Coordinator.
    Migrated from tests/fixtures/guest_apps/coordinator_stress/smoke_test.sh
    """
    from tools.testing.env import WORKSPACE_ROOT
    from tools.testing.virtmcu_test_suite.conftest_core import ManagedSubprocess

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

    python_cmd = sys.executable

    # 1. Run comprehensive test suite
    logger.info("Running complete_test.py...")
    script1 = Path(workspace_root) / "tests/fixtures/guest_apps/coordinator_stress/complete_test.py"
    async with ManagedSubprocess("complete_test", [python_cmd, "-u", str(script1)], env=env) as proc:
        assert proc.proc is not None
        rc = await proc.proc.wait()
        assert rc == 0, "complete_test.py failed"

    # 2. Run malformed packet survival test
    logger.info("Running repro_crash.py...")
    script2 = Path(workspace_root) / "tests/fixtures/guest_apps/coordinator_stress/repro_crash.py"
    async with ManagedSubprocess("repro_crash", [python_cmd, "-u", str(script2)], env=env) as proc:
        assert proc.proc is not None
        rc = await proc.proc.wait()
        assert rc == 0, "repro_crash.py failed"

    # 3. Run stress test
    logger.info("Running stress_test.py...")
    script3 = Path(workspace_root) / "tests/fixtures/guest_apps/coordinator_stress/stress_test.py"
    async with ManagedSubprocess("stress_test", [python_cmd, "-u", str(script3)], env=env) as proc:
        assert proc.proc is not None
        rc = await proc.proc.wait()
        assert rc == 0, "stress_test.py failed"

    # Check if coordinator is still alive
    assert deterministic_coordinator.returncode is None
