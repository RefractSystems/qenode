import subprocess
from pathlib import Path

import pytest


@pytest.mark.asyncio
async def test_telemetry_stress_queue(qemu_launcher, zenoh_router: str, tmp_path):
    workspace_root = Path(__file__).resolve().parent.parent
    yaml_file = workspace_root / "tests/fixtures/guest_apps/actuator/board.yaml"
    tmp_yaml = tmp_path / "board.yaml"
    dtb = tmp_path / "board.dtb"

    yaml_content = yaml_file.read_text().replace("tcp/127.0.0.1:7450", zenoh_router)
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
