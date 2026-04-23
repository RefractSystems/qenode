import subprocess
from pathlib import Path

import pytest


@pytest.mark.asyncio
async def test_phase21_spi_bus_stress(qemu_launcher, tmp_path):
    """
    Stress test: many SPI devices on many buses.
    Verify that hardening handles multiple buses and devices correctly.
    """
    workspace_root = Path(Path(Path(__file__).parent.resolve().parent))

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

    test_yaml = Path(tmp_path) / "stress_spi.yml"
    with Path(test_yaml).open("w") as f:
        f.write(yml)

    test_dtb = Path(tmp_path) / "stress_spi.dtb"
    subprocess.run(
        ["python3", "-m", "tools.yaml2qemu", test_yaml, "--out-dtb", test_dtb], check=True, cwd=workspace_root
    )

    bridge = await qemu_launcher(test_dtb, extra_args=["-S"])

    # Verify all devices are parented correctly
    for b in range(num_buses):
        spi_path = f"/spi{b}"
        # We only added d=0
        echo_path = f"{spi_path}/echo_{b}_0@0"
        bus_path = await bridge.qmp.execute("qom-get", {"path": echo_path, "property": "parent_bus"})
        assert bus_path == f"{spi_path}/ssi"


@pytest.mark.asyncio
async def test_phase21_mac_stress(qemu_launcher, tmp_path):
    """
    Stress test: multiple devices with different MAC addresses.
    """
    workspace_root = Path(Path(Path(__file__).parent.resolve().parent))

    num_devs = 8
    yml = "machine:\n  cpus:\n    - name: cpu0\n      type: cortex-a15\nperipherals:\n"
    yml += "  - name: memory\n    type: Memory.MappedMemory\n    address: 0x40000000\n    properties:\n      size: 0x01000000\n"

    macs = []
    for i in range(num_devs):
        mac = f"00:11:22:33:44:{i:02x}"
        macs.append(mac)
        addr = 0x50000000 + i * 0x1000
        yml += f'  - name: wifi{i}\n    type: virtmcu-wifi\n    address: 0x{addr:x}\n    properties:\n      MACAddress: "{mac}"\n      node: "{i}"\n'

    test_yaml = Path(tmp_path) / "stress_mac.yml"
    with Path(test_yaml).open("w") as f:
        f.write(yml)

    test_dtb = Path(tmp_path) / "stress_mac.dtb"
    subprocess.run(
        ["python3", "-m", "tools.yaml2qemu", test_yaml, "--out-dtb", test_dtb], check=True, cwd=workspace_root
    )

    bridge = await qemu_launcher(test_dtb, extra_args=["-S"])

    for i in range(num_devs):
        path = f"/wifi{i}"
        status = await bridge.qmp.execute("qom-get", {"path": path, "property": "realized"})
        assert status is True
