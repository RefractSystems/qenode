from pathlib import Path

import pytest


@pytest.mark.asyncio
async def test_yaml_platform_boot(qemu_launcher, tmp_path):
    """
    Phase 3.5: YAML platform boot test.
    Verify that a platform defined in YAML can boot and print "HI".
    """
    workspace_root = Path(__file__).resolve().parent.parent
    yaml_file = workspace_root / "test/phase3/test_board.yaml"
    kernel = workspace_root / "test/phase1/hello.elf"

    if not kernel.exists():
        import subprocess

        subprocess.run(["make", "-C", "test/phase1"], check=True, cwd=workspace_root)

    dtb = tmp_path / "test_board.dtb"
    import subprocess

    subprocess.run(
        ["uv", "run", "python3", "-m", "tools.yaml2qemu", str(yaml_file), "--out-dtb", str(dtb)],
        check=True,
        cwd=workspace_root,
    )

    bridge = await qemu_launcher(dtb, kernel, extra_args=["-S"])
    await bridge.start_emulation()

    assert await bridge.wait_for_line_on_uart("HI", timeout=5.0)
