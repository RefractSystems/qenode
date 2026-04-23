import subprocess
from pathlib import Path

import pytest


@pytest.mark.asyncio
async def test_spi_topology_integrity(qemu_launcher, tmp_path):
    """
    Task 21.7.3: Verify via QMP that child peripherals are correctly linked to their parent buses.
    """
    workspace_root = Path(Path(Path(__file__).parent.resolve().parent))

    test_yaml = tmp_path / "test_spi_topology.yml"
    with Path(test_yaml).open("w") as f:
        f.write("""
machine:
  cpus:
    - name: cpu0
      type: cortex-a15
peripherals:
  - name: memory
    type: Memory.MappedMemory
    address: 0x40000000
    properties:
      size: 0x01000000
  - name: spi0
    type: SPI.PL022
    address: 0x10000000
    properties:
      size: 0x1000
  - name: my_spi_echo
    type: spi-echo
    parent: spi0
    address: 0
""")
    test_dtb = tmp_path / "test_spi_topology.dtb"

    subprocess.run(
        ["python3", "-m", "tools.yaml2qemu", test_yaml, "--out-dtb", test_dtb], check=True, cwd=workspace_root
    )

    # Boot QEMU
    bridge = await qemu_launcher(test_dtb, extra_args=["-S"])

    # Find my_spi_echo. In arm-generic-fdt it's likely a child of its parent node.
    # Root nodes are named <name>.
    # spi0 is at root.
    spi0_path = "/spi0"
    echo_path = f"{spi0_path}/my_spi_echo@0"

    # Verify paths exist
    await bridge.qmp.execute("qom-list", {"path": spi0_path})
    await bridge.qmp.execute("qom-list", {"path": echo_path})

    # Now check its parent_bus
    bus_path = await bridge.qmp.execute("qom-get", {"path": echo_path, "property": "parent_bus"})
    assert bus_path is not None, "my_spi_echo has no parent_bus"
    assert bus_path == f"{spi0_path}/ssi", f"Unexpected parent_bus: {bus_path}"

    # Verify the bus is an SSI bus
    bus_type = await bridge.qmp.execute("qom-get", {"path": bus_path, "property": "type"})
    assert "ssi" in bus_type.lower()
