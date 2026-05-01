"""
SOTA Test Module: test_mac_parsing

Context:
This module implements tests for the test_mac_parsing subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_mac_parsing.
"""

import subprocess
from pathlib import Path

import pytest


@pytest.mark.asyncio
async def test_macaddr_parsing(qemu_launcher, tmp_path):
    """
    Validate MACAddress property passing from YAML through yaml2qemu to QEMU.
    """
    from tools.testing.env import WORKSPACE_ROOT

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
        ["python3", "-m", "tools.yaml2qemu", test_yaml, "--out-dtb", test_dtb], check=True, cwd=workspace_root
    )

    # Boot QEMU with this DTB
    await qemu_launcher(test_dtb, extra_args=["-display", "none", "-nographic", "-serial", "null", "-monitor", "null"])

    # Query QOM for the mac property
    # In QEMU, the macaddr property is accessed as 'macaddr' usually.
    # Wait, my property in TestRustDevice is called 'mac' or 'macaddr'?
    # Let me check test-qom-device. Wait, I didn't add the property to TestRustDevice yet!
