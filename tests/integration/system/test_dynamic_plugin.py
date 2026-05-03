"""
smoke test: Dynamic plugin loading.
Verify that rust-dummy and educational-dummy are correctly registered in QOM.
"""

from __future__ import annotations

import shutil
import subprocess
from collections.abc import Callable, Coroutine
from pathlib import Path
from typing import Any

import pytest

from tools.testing.env import WORKSPACE_ROOT


@pytest.mark.asyncio
async def test_dynamic_plugin(inspection_bridge: Callable[..., Coroutine[Any, Any, Any]]) -> None:

    workspace_root = WORKSPACE_ROOT
    dtb = Path(workspace_root) / "tests/fixtures/guest_apps/boot_arm/minimal.dtb"
    kernel = Path(workspace_root) / "tests/fixtures/guest_apps/boot_arm/hello.elf"

    # 1. Build if missing (crucial for CI robustness)
    if not Path(dtb).exists() or not Path(kernel).exists():
        subprocess.run(
            [shutil.which("make") or "make", "-C", "tests/fixtures/guest_apps/boot_arm"],
            check=True,
            cwd=workspace_root,
        )

    bridge = await inspection_bridge(dtb, extra_args=["-device", "rust-dummy", "-device", "dummy-device"])

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
