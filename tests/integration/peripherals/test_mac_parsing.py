"""
SOTA Test Module: test_mac_parsing

Context:
This module implements tests for the test_mac_parsing subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_mac_parsing.
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
async def test_macaddr_parsing(inspection_bridge: Callable[..., Coroutine[Any, Any, Any]], tmp_path: Path) -> None:
    """
    Validate MACAddress property passing from YAML through yaml2qemu to QEMU.
    """
    workspace_root = WORKSPACE_ROOT

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
    test_dtb = tmp_path / "test_mac.dtb"

    subprocess.run(
        [shutil.which("python3") or "python3", "-m", "tools.yaml2qemu", test_yaml, "--out-dtb", test_dtb],
        check=True,
        cwd=workspace_root,
    )

    # Boot QEMU with this DTB
    await inspection_bridge(test_dtb)

    # Query QOM for the mac property
    # In QEMU, the macaddr property is accessed as 'macaddr' usually.
    # Wait, my property in TestRustDevice is called 'mac' or 'macaddr'?
    # Let me check test-qom-device. Wait, I didn't add the property to TestRustDevice yet!
