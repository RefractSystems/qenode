"""
smoke test: Dynamic plugin loading.
Verify that rust-dummy and educational-dummy are correctly registered in QOM.
"""

from __future__ import annotations

from collections.abc import Callable, Coroutine
from typing import Any

import pytest


@pytest.mark.asyncio
async def test_dynamic_plugin(inspection_bridge: Callable[..., Coroutine[Any, Any, Any]], guest_app_factory: Any) -> None:  # noqa: ANN401  # noqa: ANN401
    app_dir = guest_app_factory("boot_arm")
    dtb = app_dir / "minimal.dtb"
    app_dir / "hello.elf"

    # 1. Build (handled by guest_app_factory)

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
