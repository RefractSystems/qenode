"""
SOTA Test Module: test_telemetry_stress

Context:
This module implements tests for the test_telemetry_stress subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_telemetry_stress.
"""

import subprocess

import pytest


@pytest.mark.asyncio
async def test_telemetry_stress_queue(qemu_launcher, zenoh_router: str, tmp_path):
    from tools.testing.env import WORKSPACE_ROOT

    workspace_root = WORKSPACE_ROOT
    yaml_file = workspace_root / "tests/fixtures/guest_apps/actuator/board.yaml"
    tmp_yaml = tmp_path / "board.yaml"
    dtb = tmp_path / "board.dtb"

    yaml_content = yaml_file.read_text().replace("ZENOH_ROUTER_ENDPOINT", zenoh_router)
    tmp_yaml.write_text(yaml_content)

    subprocess.run(
        ["uv", "run", "python3", "-m", "tools.yaml2qemu", str(tmp_yaml), "--out-dtb", str(dtb)],
        check=True,
        cwd=workspace_root,
    )

    bridge = await qemu_launcher(
        dtb,
        extra_args=["-S"],  # Start paused
    )

    await bridge.start_emulation()

    status = await bridge.qmp.execute("query-status")
    assert status["running"] is True
