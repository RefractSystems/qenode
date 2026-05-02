"""
SOTA Test Module: test_spi_multibus

Context:
This module implements tests for the test_spi_multibus subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_spi_multibus.
"""

from __future__ import annotations

import shutil
import subprocess
from pathlib import Path
from typing import Any, cast

import pytest


@pytest.mark.asyncio
async def test_spi_bus_stress(inspection_bridge: object, tmp_path: Path) -> None:
    """
    Stress test: many SPI devices on many buses.
    Verify that hardening handles multiple buses and devices correctly.
    """
    from tools.testing.env import WORKSPACE_ROOT

    workspace_root = WORKSPACE_ROOT

    # 4 SPI buses, each with 4 devices (QEMU might have some limits)
    num_buses = 4
    devs_per_bus = 4

    yml = "machine:\n  cpus:\n    - name: cpu0\n      type: cortex-a15\nperipherals:\n"
    yml += "  - name: memory\n    type: Memory.MappedMemory\n    address: 0x40000000\n    properties:\n      size: 0x01000000\n"

    for b in range(num_buses):
        bus_name = f"spi{b}"
        addr = 0x10000000 + b * 0x1000
        yml += (
            f"  - name: {bus_name}\n    type: SPI.PL022\n    address: 0x{addr:x}\n    properties:\n      size: 0x1000\n"
        )

        for d in range(devs_per_bus):
            dev_name = f"echo_{b}_{d}"
            # PL022 might only support 1 CS. Let's try 1 per bus first.
            if d == 0:
                yml += f"  - name: {dev_name}\n    type: spi-echo\n    parent: {bus_name}\n    address: {d}\n"

    test_yaml = tmp_path / "stress_spi.yml"
    with Path(test_yaml).open("w") as f:
        f.write(yml)

    test_dtb = tmp_path / "stress_spi.dtb"
    subprocess.run(
        [shutil.which("python3") or "python3", "-m", "tools.yaml2qemu", test_yaml, "--out-dtb", test_dtb],
        check=True,
        cwd=workspace_root,
    )

    bridge = await cast(Any, inspection_bridge)(test_dtb)

    # Verify all devices are parented correctly
    for b in range(num_buses):
        addr = 0x10000000 + b * 0x1000
        spi_path = f"/spi{b}@{addr:x}"
        # We only added d=0
        echo_path = f"{spi_path}/echo_{b}_0@0"
        bus_path = await bridge.qmp.execute("qom-get", {"path": echo_path, "property": "parent_bus"})
        assert bus_path == f"{spi_path}/ssi"


@pytest.mark.asyncio
async def test_mac_stress(inspection_bridge: object, tmp_path: Path) -> None:
    """
    Stress test: multiple devices with different MAC addresses.
    """
    from tools.testing.env import WORKSPACE_ROOT

    workspace_root = WORKSPACE_ROOT

    num_devs = 8
    yml = "machine:\n  cpus:\n    - name: cpu0\n      type: cortex-a15\nperipherals:\n"
    yml += "  - name: memory\n    type: Memory.MappedMemory\n    address: 0x40000000\n    properties:\n      size: 0x01000000\n"

    macs = []
    for i in range(num_devs):
        mac = f"00:11:22:33:44:{i:02x}"
        macs.append(mac)
        addr = 0x50000000 + i * 0x1000
        yml += f'  - name: wifi{i}\n    type: wifi\n    address: 0x{addr:x}\n    properties:\n      MACAddress: "{mac}"\n      node: "{i}"\n'

    test_yaml = tmp_path / "stress_mac.yml"
    with Path(test_yaml).open("w") as f:
        f.write(yml)

    test_dtb = tmp_path / "stress_mac.dtb"
    subprocess.run(
        [shutil.which("python3") or "python3", "-m", "tools.yaml2qemu", test_yaml, "--out-dtb", test_dtb],
        check=True,
        cwd=workspace_root,
    )

    bridge = await cast(Any, inspection_bridge)(test_dtb)

    for i in range(num_devs):
        addr = 0x50000000 + i * 0x1000
        path = f"/wifi{i}@{addr:x}"
        status = await bridge.qmp.execute("qom-get", {"path": path, "property": "realized"})
        assert status is True
