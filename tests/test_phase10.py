import asyncio
from pathlib import Path

import pytest


@pytest.mark.asyncio
async def test_phase10_usd_metadata():
    """
    TEST 1: OpenUSD Metadata Tool
    """
    workspace_root = Path(Path(Path(__file__).parent.resolve().parent))
    yaml_file = Path(workspace_root) / "test/phase3/test_board.yaml"
    tool = Path(workspace_root) / "tools/usd_to_virtmcu.py"

    proc = await asyncio.create_subprocess_exec(
        "python3", tool, yaml_file, stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE
    )
    stdout, _stderr = await proc.communicate()
    assert proc.returncode == 0
    output = stdout.decode()
    assert "MEMORY_BASE" in output
    assert "UART0_BASE" in output
    assert "GIC_BASE" in output


@pytest.mark.asyncio
async def test_phase10_resd_replay_startup():
    """
    TEST 3: resd_replay startup + empty-file rejection
    """
    workspace_root = Path(Path(Path(__file__).parent.resolve().parent))
    replay_bin = workspace_root / "target/release/resd_replay"
    if not replay_bin.exists():
        replay_bin = workspace_root / "tools/cyber_bridge/target/release/resd_replay"

    # Missing file should fail
    proc = await asyncio.create_subprocess_exec(
        str(replay_bin), "/nonexistent.resd", "0", stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE
    )

    _stdout, stderr = await proc.communicate()
    assert proc.returncode != 0
    assert b"Failed to parse" in stderr


@pytest.mark.asyncio
async def test_phase10_mujoco_bridge_shm():
    """
    TEST 4: mujoco_bridge shared memory creation
    """
    workspace_root = Path(Path(Path(__file__).parent.resolve().parent))
    bridge_bin = workspace_root / "target/release/mujoco_bridge"
    if not bridge_bin.exists():
        bridge_bin = workspace_root / "tools/cyber_bridge/target/release/mujoco_bridge"

    node_id = 99
    shm_path = f"/dev/shm/virtmcu_mujoco_{node_id}"
    if Path(shm_path).exists():
        Path(shm_path).unlink()

    # Start bridge
    proc = await asyncio.create_subprocess_exec(
        str(bridge_bin), str(node_id), "2", "6", stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE
    )
    # Wait for SHM to appear
    for _ in range(20):
        if Path(shm_path).exists():
            break
        await asyncio.sleep(0.1)

    assert Path(shm_path).exists(), "Shared memory segment not created"

    # Cleanup
    proc.terminate()
    await proc.wait()
    if Path(shm_path).exists():
        Path(shm_path).unlink()
