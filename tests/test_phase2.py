from pathlib import Path

import pytest


@pytest.mark.asyncio
async def test_phase2_dynamic_plugin(qemu_launcher):
    """
    Phase 2 smoke test: Dynamic plugin loading.
    Verify that rust-dummy and educational-dummy are correctly registered in QOM.
    """
    import subprocess

    workspace_root = Path(Path(Path(__file__).parent.resolve().parent))
    dtb = Path(workspace_root) / "tests/fixtures/guest_apps/phase1/minimal.dtb"
    kernel = Path(workspace_root) / "tests/fixtures/guest_apps/phase1/hello.elf"

    # 1. Build if missing (crucial for CI robustness)
    if not Path(dtb).exists() or not Path(kernel).exists():
        subprocess.run(["make", "-C", "tests/fixtures/guest_apps/phase1"], check=True, cwd=workspace_root)

    bridge = await qemu_launcher(dtb, extra_args=["-device", "rust-dummy", "-device", "dummy-device"])

    # Check QOM tree for the devices
    res = await bridge.qmp.execute("qom-list", {"path": "/machine/peripheral-anon"})

    found_rust = False
    found_c = False
    for item in res:
        if item.get("type") == "child<rust-dummy>":
            found_rust = True
        elif item.get("type") == "child<dummy-device>":
            found_c = True

    assert found_rust, f"rust-dummy not found in QOM tree: {res}"
    assert found_c, f"dummy-device (educational C module) not found in QOM tree: {res}"
