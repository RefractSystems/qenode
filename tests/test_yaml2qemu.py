"""
tests/test_yaml2qemu.py — Unit tests for tools/yaml2qemu.py

Tests the YAML platform description parser in isolation (no QEMU binary needed).
Covers CPU mapping, peripheral mapping, interrupt parsing, and edge cases.
"""

import os
import sys
import tempfile

import yaml

sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "..")))
from tools.repl2qemu.parser import ReplPlatform
from tools.yaml2qemu import parse_yaml_platform

# ── Helpers ───────────────────────────────────────────────────────────────────


def write_yaml(data: dict) -> str:
    """Write a temporary YAML file, return its path. Caller must unlink."""
    f = tempfile.NamedTemporaryFile(mode="w", suffix=".yaml", delete=False)
    yaml.dump(data, f)
    f.close()
    return f.name


# ── CPU mapping ───────────────────────────────────────────────────────────────


def test_parse_single_cpu():
    path = write_yaml(
        {
            "machine": {"cpus": [{"name": "cpu0", "type": "cortex-a15"}]},
            "peripherals": [],
        }
    )
    try:
        platform, _ = parse_yaml_platform(path)
        assert len(platform.devices) == 1
        dev = platform.devices[0]
        assert dev.name == "cpu0"
        assert dev.type_name == "CPU.ARMv7A"
        assert dev.properties["cpuType"] == "cortex-a15"
    finally:
        os.unlink(path)


def test_parse_multi_cpu():
    path = write_yaml(
        {
            "machine": {
                "cpus": [
                    {"name": "cpu0", "type": "cortex-a15"},
                    {"name": "cpu1", "type": "cortex-a15"},
                ]
            },
            "peripherals": [],
        }
    )
    try:
        platform, _ = parse_yaml_platform(path)
        assert len(platform.devices) == 2
        names = {d.name for d in platform.devices}
        assert names == {"cpu0", "cpu1"}
    finally:
        os.unlink(path)


# ── Peripheral mapping ────────────────────────────────────────────────────────


def test_parse_uart_peripheral():
    path = write_yaml(
        {
            "machine": {"cpus": []},
            "peripherals": [
                {
                    "name": "uart0",
                    "type": "UART.PL011",
                    "address": "0x09000000",
                }
            ],
        }
    )
    try:
        platform, _ = parse_yaml_platform(path)
        devs = [d for d in platform.devices if d.name == "uart0"]
        assert len(devs) == 1
        assert devs[0].type_name == "UART.PL011"
        assert devs[0].address_str == "0x09000000"
    finally:
        os.unlink(path)


def test_parse_memory_with_properties():
    path = write_yaml(
        {
            "machine": {"cpus": []},
            "peripherals": [
                {
                    "name": "sram",
                    "type": "Memory.MappedMemory",
                    "address": "0x20000000",
                    "properties": {"size": "0x00040000"},
                }
            ],
        }
    )
    try:
        platform, _ = parse_yaml_platform(path)
        dev = next(d for d in platform.devices if d.name == "sram")
        assert dev.type_name == "Memory.MappedMemory"
        assert dev.properties["size"] == "0x00040000"
    finally:
        os.unlink(path)


def test_parse_interrupt():
    path = write_yaml(
        {
            "machine": {"cpus": []},
            "peripherals": [
                {
                    "name": "usart1",
                    "type": "UART.STM32_UART",
                    "address": "0x40011000",
                    "interrupts": ["nvic@37"],
                }
            ],
        }
    )
    try:
        platform, _ = parse_yaml_platform(path)
        dev = next(d for d in platform.devices if d.name == "usart1")
        assert len(dev.interrupts) == 1
        irq = dev.interrupts[0]
        assert irq.target_device == "nvic"
        assert irq.target_range == "37"
    finally:
        os.unlink(path)


def test_renode_type_alias():
    """Files migrated from .repl may use 'renode_type' instead of 'type'."""
    path = write_yaml(
        {
            "machine": {"cpus": []},
            "peripherals": [{"name": "uart0", "renode_type": "UART.PL011", "address": "0x09000000"}],
        }
    )
    try:
        platform, _ = parse_yaml_platform(path)
        dev = next(d for d in platform.devices if d.name == "uart0")
        assert dev.type_name == "UART.PL011"
    finally:
        os.unlink(path)


def test_empty_platform():
    path = write_yaml({"machine": {"cpus": []}, "peripherals": []})
    try:
        platform, _ = parse_yaml_platform(path)
        assert isinstance(platform, ReplPlatform)
        assert len(platform.devices) == 0
    finally:
        os.unlink(path)


def test_cpu_and_peripherals_combined():
    path = write_yaml(
        {
            "machine": {"cpus": [{"name": "cpu0", "type": "cortex-a15"}]},
            "peripherals": [
                {"name": "sram", "type": "Memory.MappedMemory", "address": "0x20000000"},
                {"name": "uart0", "type": "UART.PL011", "address": "0x09000000"},
            ],
        }
    )
    try:
        platform, _ = parse_yaml_platform(path)
        # 1 CPU + 2 peripherals
        assert len(platform.devices) == 3
        names = {d.name for d in platform.devices}
        assert names == {"cpu0", "sram", "uart0"}
    finally:
        os.unlink(path)
