import pytest


@pytest.mark.asyncio
async def test_yaml_platform_boot(simulation, tmp_path):
    """
    YAML platform boot test.
    Verify that a platform defined in YAML can boot and print "HI".
    """
    from tools.testing.env import WORKSPACE_ROOT

    workspace_root = WORKSPACE_ROOT
    yaml_file = workspace_root / "tests/fixtures/guest_apps/yaml_boot/test_board.yaml"
    kernel = workspace_root / "tests/fixtures/guest_apps/boot_arm/hello.elf"

    if not kernel.exists():
        import subprocess

        subprocess.run(["make", "-C", "tests/fixtures/guest_apps/boot_arm"], check=True, cwd=workspace_root)

    dtb = tmp_path / "test_board.dtb"
    import subprocess

    subprocess.run(
        ["uv", "run", "python3", "-m", "tools.yaml2qemu", str(yaml_file), "--out-dtb", str(dtb)],
        check=True,
        cwd=workspace_root,
    )

    # Boot and check UART using VirtmcuSimulation
    async with await simulation(dtb, kernel) as sim:
        await sim.vta.step(100_000_000)
        assert await sim.bridge.wait_for_line_on_uart("HI", timeout=5.0)
