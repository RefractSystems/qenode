"""
SOTA Test Module: test_parser

Context:
This module implements tests for the test_parser subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_parser.
"""

from __future__ import annotations

import logging
import sys
from pathlib import Path
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from _pytest.logging import LogCaptureFixture


from tools.repl2qemu.parser import parse_repl


def test_parse_simple_memory() -> None:
    repl = """
sram: Memory.MappedMemory @ sysbus 0x20000000
    size: 0x00040000
"""
    platform = parse_repl(repl)
    assert len(platform.devices) == 1
    dev = platform.devices[0]
    assert dev.name == "sram"
    assert dev.type_name == "Memory.MappedMemory"
    assert dev.address_str == "0x20000000"
    assert dev.properties["size"] == "0x00040000"
    assert len(dev.interrupts) == 0


def test_parse_device_with_irq() -> None:
    repl = """
usart1: UART.STM32_UART @ sysbus <0x40011000, +0x100>
    -> nvic@37
"""
    platform = parse_repl(repl)
    assert len(platform.devices) == 1
    dev = platform.devices[0]
    assert dev.name == "usart1"
    assert dev.type_name == "UART.STM32_UART"
    assert dev.address_str == "<0x40011000, +0x100>"
    assert len(dev.interrupts) == 1
    irq = dev.interrupts[0]
    assert irq.target_device == "nvic"
    assert irq.target_range == "37"


def test_parse_ranged_irq() -> None:
    repl = """
can1: CAN.STMCAN @ sysbus <0x40006400, +0x400>
    [0-3] -> nvic@[19-22]
"""
    platform = parse_repl(repl)
    assert len(platform.devices) == 1
    dev = platform.devices[0]
    assert len(dev.interrupts) == 1
    irq = dev.interrupts[0]
    assert irq.source_range == "0-3"
    assert irq.target_device == "nvic"
    assert irq.target_range == "19-22"


def test_parse_inline_block() -> None:
    repl = """
flash_controller: MTD.STM32F4_FlashController @ {
        sysbus 0x40023C00;
        sysbus new Bus.BusMultiRegistration { address: 0x1FFFC000; size: 0x100; region: "optionBytes" }
    }
    flash: flash
"""
    platform = parse_repl(repl)
    assert len(platform.devices) == 1
    dev = platform.devices[0]
    assert dev.name == "flash_controller"
    assert dev.type_name == "MTD.STM32F4_FlashController"
    # we don't strictly parse the inline block yet, but we shouldn't crash
    assert dev.properties["flash"] == "flash"


def test_parse_comments() -> None:
    repl = """
// This is a comment
usart1: UART.STM32_UART @ sysbus 0x40011000 // Inline comment
    size: 0x100 // Property comment
"""
    platform = parse_repl(repl)
    assert len(platform.devices) == 1
    dev = platform.devices[0]
    assert dev.name == "usart1"
    assert dev.properties["size"] == "0x100"


def test_parse_multiline_properties() -> None:
    # Renode properties can sometimes span multiple lines or be in blocks
    repl = """
cpu: CPU.CortexM @ sysbus
    cpuType: "cortex-m4"
    nvic: nvic
    priorityMask: 0xFF
"""
    platform = parse_repl(repl)
    assert len(platform.devices) == 1
    dev = platform.devices[0]
    assert dev.properties["cpuType"] == "cortex-m4"
    assert dev.properties["nvic"] == "nvic"


def test_parse_using_statement(tmp_path: Path) -> None:
    child_repl = tmp_path / "child.repl"
    child_repl.write_text("usart1: UART.STM32_UART @ sysbus 0x40011000\n")

    parent_repl = tmp_path / "parent.repl"
    parent_repl.write_text(f'using "{child_repl.name}"\n')

    platform = parse_repl(parent_repl.read_text(), str(tmp_path))
    assert len(platform.devices) == 1
    assert platform.devices[0].name == "usart1"


def test_parse_recursive_using(tmp_path: Path) -> None:
    a_repl = tmp_path / "a.repl"
    b_repl = tmp_path / "b.repl"
    c_repl = tmp_path / "c.repl"

    a_repl.write_text('using "b.repl"\ndevA: CPU.CortexM\n')
    b_repl.write_text('using "c.repl"\ndevB: CPU.CortexM\n')
    c_repl.write_text("devC: CPU.CortexM\n")

    platform = parse_repl(a_repl.read_text(), str(tmp_path))
    assert len(platform.devices) == 3
    names = [d.name for d in platform.devices]
    assert set(names) == {"devA", "devB", "devC"}


def test_stress_test_parser() -> None:
    lines = []
    for i in range(1000):
        lines.append(f"dev{i}: CPU.CortexM @ sysbus {hex(0x1000 * i)}")
        lines.append(f"    prop: {i}")
        lines.append(f"    -> nvic@{i % 100}")

    repl = "\n".join(lines)
    platform = parse_repl(repl)
    assert len(platform.devices) == 1000
    assert platform.devices[999].name == "dev999"


def test_parser_main(tmp_path: Path) -> None:
    import subprocess

    from tools.testing.env import WORKSPACE_DIR

    repl_file = tmp_path / "test.repl"
    repl_file.write_text("sram: Memory.MappedMemory @ sysbus 0x20000000\n")

    result = subprocess.run(
        [sys.executable, "-m", "tools.repl2qemu.parser", str(repl_file)],
        capture_output=True,
        text=True,
        cwd=WORKSPACE_DIR,
    )
    assert result.returncode == 0
    assert "sram" in result.stdout


def test_parse_complex_attributes() -> None:
    repl = """
button: Miscellaneous.Button @ gpioPortA 0
    -> gpioPortA@0
    invert: true
"""
    platform = parse_repl(repl)
    assert len(platform.devices) == 1
    dev = platform.devices[0]
    assert dev.name == "button"
    assert dev.address_str == "gpioPortA 0"


def test_parse_nested_blocks() -> None:
    repl = """
sysbus:
    init:
        Tag 0x40023800 0x400 "RCC"
"""
    platform = parse_repl(repl)
    # sysbus is not a device in the traditional sense, but current regex might catch it
    for dev in platform.devices:
        assert dev.name != "sysbus"


def test_parser_missing_using(caplog: LogCaptureFixture) -> None:
    repl = 'using "non_existent.repl"\n'
    parse_repl(repl)
    caplog.set_level(logging.INFO)
    assert "Warning: Included file not found" in caplog.text


def test_parser_sysbus_registration() -> None:
    # Test the multi-line block parsing for address:
    repl = """
flash_controller: MTD.STM32F4_FlashController @ {
    sysbus 0x40023C00;
    sysbus new BusMultiRegistration { address: 0x1FFFC000; size: 0x100; region: "optionBytes" }
}
"""
    platform = parse_repl(repl)
    assert len(platform.devices) == 1
    assert platform.devices[0].address_str == "0x1FFFC000"


def test_parser_addr_trailing_at() -> None:
    # Hit line 78
    repl = "usart1: UART.STM32_UART @ sysbus 0x40011000@"
    platform = parse_repl(repl)
    assert platform.devices[0].address_str == "0x40011000"


def test_parser_standalone_block_start() -> None:
    # Hit line 96-97
    repl = """
usart1: UART.STM32_UART @ sysbus 0x40011000
{
    // block
}
"""
    platform = parse_repl(repl)
    assert len(platform.devices) == 1
