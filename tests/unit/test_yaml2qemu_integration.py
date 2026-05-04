"""
tests/test_yaml2qemu_integration.py — Integration tests for tools/yaml2qemu.py

Tests the full flow from YAML input to generated DTB/DTS, ensuring that
filtering logic, property injection, and validation all work together.
"""

from __future__ import annotations

import typing
from pathlib import Path

import yaml

from tools.testing.virtmcu_test_suite.factory import compile_yaml, inspect_dtb

# ── Helpers ───────────────────────────────────────────────────────────────────


def run_yaml2qemu(yaml_data: dict[typing.Any, typing.Any], tmp_path: Path) -> tuple[str, str]:
    """
    Runs yaml2qemu.py on the provided data and returns (dts_content, cli_content).
    """
    yaml_file = tmp_path / "board.yaml"
    dtb_file = tmp_path / "board.dtb"
    cli_file = tmp_path / "board.cli"

    yaml_file.write_text(yaml.dump(yaml_data))

    # Run the tool via SOTA factory
    compile_yaml(yaml_file, dtb_file, out_cli=cli_file)

    # Decompile DTB to DTS for inspection
    dts_content = inspect_dtb(dtb_file)
    cli_content = cli_file.read_text()

    return dts_content, cli_content


# ── Tests ─────────────────────────────────────────────────────────────────────


def test_roundtrip_basic_arm(tmp_path: Path) -> None:
    """Verify a simple ARM platform with UART and Memory."""
    data = {
        "machine": {"cpus": [{"name": "cpu0", "type": "cortex-a15"}]},
        "memory": [{"name": "ram", "address": 0x40000000, "size": 0x1000000}],
        "peripherals": [
            {"name": "gic", "type": "IRQControllers.GIC", "address": 0x08000000},
            {"name": "uart0", "type": "UART.PL011", "address": 0x09000000, "interrupts": [33]},
        ],
    }
    dts, _ = run_yaml2qemu(data, tmp_path)

    assert 'compatible = "arm,generic-fdt";' in dts
    assert "cpu0@0" in dts
    assert "memory@40000000" in dts
    assert "uart0@9000000 {" in dts
    assert 'compatible = "pl011";' in dts
    assert "interrupts = <0x00 0x21 0x04>;" in dts  # GIC format: SPI 33, level-high


def test_roundtrip_wireless_devices(tmp_path: Path) -> None:
    """
    Verify that wireless devices (telemetry, ieee802154) are correctly
    included in DTB and have the transport property injected.
    (This would have caught the regression).
    """
    data = {
        "machine": {"cpus": [{"name": "cpu0", "type": "cortex-a15"}]},
        "peripherals": [
            {
                "name": "radio0",
                "type": "ieee802154",
                "address": 0x9001000,
                "properties": {"node": 0},
            },
            {
                "name": "tele0",
                "type": "telemetry",
                "address": 0x9002000,
                "properties": {"node": 0},
            },
        ],
    }
    dts, _ = run_yaml2qemu(data, tmp_path)

    # Check radio0
    assert "radio0@9001000 {" in dts
    assert 'compatible = "ieee802154";' in dts
    assert 'transport = "zenoh";' in dts
    assert "node = <0x00>;" in dts

    # Check tele0
    assert "tele0@9002000 {" in dts
    assert 'compatible = "telemetry";' in dts
    assert 'transport = "zenoh";' in dts


def test_roundtrip_chardev_cli_only(tmp_path: Path) -> None:
    """Verify that 'chardev' type only goes to CLI and NOT to DTB."""
    data = {
        "machine": {"cpus": [{"name": "cpu0", "type": "cortex-a15"}]},
        "peripherals": [
            {
                "name": "serial0",
                "type": "chardev",
                "properties": {"id": "my_serial", "node": 1},
            }
        ],
    }
    dts, cli = run_yaml2qemu(data, tmp_path)

    assert "serial0" not in dts
    assert "-chardev" in cli
    assert "virtmcu,id=my_serial,node=1,transport=zenoh" in cli


def test_roundtrip_riscv_platform(tmp_path: Path) -> None:
    """Verify RISC-V platform generation."""
    data = {
        "machine": {
            "cpus": [
                {
                    "name": "cpu0",
                    "type": "riscv64",
                    "isa": "rv64imac",
                    "mmu_type": "riscv,sv39",
                }
            ]
        },
        "memory": [{"name": "dram", "address": 0x80000000, "size": 0x1000000}],
        "peripherals": [],
    }
    dts, _ = run_yaml2qemu(data, tmp_path)

    assert 'compatible = "riscv-virtio";' in dts
    assert 'riscv,isa = "rv64imac";' in dts
    assert 'mmu-type = "riscv,sv39";' in dts
    assert "memory@80000000" in dts


def test_roundtrip_mmio_socket_bridge(tmp_path: Path) -> None:
    """Verify mmio-socket-bridge appears in DTB with its mandatory container."""
    socket_path = str(tmp_path / "test.sock")
    data = {
        "machine": {"cpus": [{"name": "cpu0", "type": "cortex-a15"}]},
        "peripherals": [
            {
                "name": "bridge0",
                "type": "mmio-socket-bridge",
                "address": 0x50000000,
                "properties": {
                    "socket-path": socket_path,
                    "region-size": 0x1000,
                },
            }
        ],
    }
    dts, _ = run_yaml2qemu(data, tmp_path)

    assert "bridge0@50000000 {" in dts
    assert 'compatible = "mmio-socket-bridge"' in dts

    assert f'socket-path = "{socket_path}";' in dts
    assert "container = <" in dts  # Must have sysmem container for MMIO access
