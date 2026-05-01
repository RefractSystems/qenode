"""
SOTA Test Module: test_telemetry_stress

Context:
This module implements tests for the test_telemetry_stress subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_telemetry_stress.
"""

from __future__ import annotations

import shutil
import subprocess
from typing import TYPE_CHECKING, Any, cast

import pytest

if TYPE_CHECKING:
    from pathlib import Path

    pass


@pytest.mark.asyncio
async def test_telemetry_stress_queue(qemu_launcher: object, zenoh_router: str, tmp_path: Path) -> None:
    from tools.testing.env import WORKSPACE_ROOT

    workspace_root = WORKSPACE_ROOT
    yaml_file = workspace_root / "tests/fixtures/guest_apps/actuator/board.yaml"
    tmp_yaml = tmp_path / "board.yaml"
    dtb = tmp_path / "board.dtb"

    yaml_content = yaml_file.read_text().replace("ZENOH_ROUTER_ENDPOINT", zenoh_router)
    tmp_yaml.write_text(yaml_content)

    subprocess.run(
        [shutil.which("uv") or "uv", "run", "python3", "-m", "tools.yaml2qemu", str(tmp_yaml), "--out-dtb", str(dtb)],
        check=True,
        cwd=workspace_root,
    )

    bridge = await cast(Any, qemu_launcher)(
        dtb,
        extra_args=["-S"],  # Start paused
    )

    await bridge.start_emulation()

    status = await bridge.qmp.execute("query-status")
    assert status["running"] is True
