"""
SOTA Test Module: test_topology_integrity

Context:
This module implements tests for the test_topology_integrity subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_topology_integrity.
"""

from __future__ import annotations

from pathlib import Path
from typing import TYPE_CHECKING, Any, cast

import pytest
import yaml

if TYPE_CHECKING:
    pass


@pytest.mark.asyncio
async def test_spi_topology_integrity(inspection_bridge: object, tmp_path: Path) -> None:
    """
    Verify via QMP that child peripherals are correctly linked to their parent buses.
    """
    from tools.testing.virtmcu_test_suite.factory import compile_yaml
    from tools.testing.virtmcu_test_suite.generated import (
        Address,
        Cpu,
        Machine,
        NodeID,
        Resource,
        World,
    )

    test_yaml = tmp_path / "test_spi_topology.yml"
    world = World(
        machine=Machine(
            cpus=[Cpu(name="cpu0", type="cortex-a15")],
        ),
        peripherals=[
            Resource(
                name=NodeID(root="memory"),
                type="Memory.MappedMemory",
                address=Address(root="0x40000000"),
                properties=cast(Any, {"size": "0x01000000"}),
            ),
            Resource(
                name=NodeID(root="spi0"),
                type="SPI.PL022",
                address=Address(root="0x10000000"),
                properties=cast(Any, {"size": "0x1000"}),
            ),
            Resource(
                name=NodeID(root="my_spi_echo"),
                type="spi-echo",
                parent="spi0",
                address=Address(root=0),
            ),
        ],
    )
    test_yaml.write_text(yaml.dump(world.model_dump(exclude_none=True, by_alias=True), sort_keys=False))
    test_dtb = tmp_path / "test_spi_topology.dtb"

    compile_yaml(test_yaml, test_dtb)

    # Boot QEMU
    bridge = await cast(Any, inspection_bridge)(test_dtb)

    # Find my_spi_echo. In arm-generic-fdt it's likely a child of its parent node.
    # Root nodes are named <name>@<address>.
    # spi0 is at root.
    spi0_path = "/spi0@10000000"
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
