"""
SOTA Test Module: test_mac_parsing

Context:
This module implements tests for the test_mac_parsing subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_mac_parsing.
"""

from __future__ import annotations

import logging
from collections.abc import Callable, Coroutine
from pathlib import Path
from typing import Any

import pytest

from tools.testing.virtmcu_test_suite.factory import compile_yaml

logger = logging.getLogger(__name__)


@pytest.mark.asyncio
async def test_macaddr_parsing(inspection_bridge: Callable[..., Coroutine[Any, Any, Any]], tmp_path: Path) -> None:
    """
    Validate MACAddress property passing from YAML through yaml2qemu to QEMU.
    """

    # We will temporarily inject a zenoh-wifi node to test macaddr parsing
    test_yaml = tmp_path / "test_mac.yml"
    with Path(test_yaml).open("w") as f:
        f.write(
            "machine:\n"
            "  cpus:\n"
            "    - name: cpu0\n"
            "      type: cortex-a15\n"
            "peripherals:\n"
            "  - name: ram\n"
            "    type: Memory.MappedMemory\n"
            "    address: 0x40000000\n"
            "    properties:\n"
            "      size: 0x1000000\n"
            "  - name: test_dev\n"
            "    type: test-rust-device\n"
            "    address: sysbus\n"
            "    properties:\n"
            '      MACAddress: "00:11:22:33:44:55"\n'
        )

    test_dtb = compile_yaml(test_yaml, tmp_path / "test_mac.dtb")

    # Boot QEMU with this DTB
    bridge = await inspection_bridge(test_dtb)

    # List all objects to find our device
    async def find_device(path: str) -> str | None:
        try:
            objs = await bridge.execute("qom-list", {"path": path})
        except Exception:
            return None

        for obj in objs:
            # In some QEMU versions, it might not have 'type' in qom-list output
            # but it has 'name'. We can check the type of the child.
            child_name = obj["name"]
            child_path = f"{path}/{child_name}" if path != "/" else f"/{child_name}"

            try:
                t = await bridge.execute("qom-get", {"path": child_path, "property": "type"})
                if t == "test-rust-device":
                    return child_path
            except Exception as e:
                logger.debug(f"Failed to get type for {child_path}: {e}")
            # Avoid infinite recursion and stay within reasonable depth
            if path.count("/") < 5:
                res = await find_device(child_path)
                if res:
                    return res
        return None

    # Search common roots
    device_path = await find_device("/machine")
    if not device_path:
        device_path = await find_device("/machine/unattached")
    if not device_path:
        device_path = await find_device("/machine/peripheral")
    if not device_path:
        device_path = await find_device("/machine/peripheral-anon")

    assert device_path is not None, "test-rust-device not found in QOM tree"

    # Query QOM for the mac property
    res = await bridge.execute(
        "qom-get",
        {"path": device_path, "property": "macaddr"},
    )
    assert res == "00:11:22:33:44:55"
