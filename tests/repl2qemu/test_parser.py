import os
import sys

sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "../../")))
from tools.repl2qemu.parser import parse_repl


def test_parse_simple_memory():
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


def test_parse_device_with_irq():
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


def test_parse_ranged_irq():
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


def test_parse_inline_block():
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


def test_parse_comments():
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


def test_parse_multiline_properties():
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


def test_parse_using_statement():
    # CURRENTLY FAILS: parser.py doesn't handle 'using'
    repl = """
using "platforms/cpus/stm32f4.repl"
usart1: UART.STM32_UART @ sysbus 0x40011000
"""
    platform = parse_repl(repl)
    # The 'using' line might be misidentified as a device or just ignored
    # If it's ignored, we only get 1 device.
    # If it's misidentified, we might get 0 or 2.
    names = [d.name for d in platform.devices]
    assert "usart1" in names
    assert "using" not in names


def test_parse_complex_attributes():
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


def test_parse_nested_blocks():
    repl = """
sysbus:
    init:
        Tag 0x40023800 0x400 "RCC"
"""
    platform = parse_repl(repl)
    # sysbus is not a device in the traditional sense, but current regex might catch it
    for dev in platform.devices:
        assert dev.name != "sysbus"


def test_parser_missing_using(capsys):
    repl = 'using "non_existent.repl"\n'
    parse_repl(repl)
    captured = capsys.readouterr()
    assert "Warning: Included file not found" in captured.out


def test_parser_sysbus_registration():
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


def test_parser_addr_trailing_at():
    # Hit line 78
    repl = "usart1: UART.STM32_UART @ sysbus 0x40011000@"
    platform = parse_repl(repl)
    assert platform.devices[0].address_str == "0x40011000"


def test_parser_standalone_block_start():
    # Hit line 96-97
    repl = """
usart1: UART.STM32_UART @ sysbus 0x40011000
{
    // block
}
"""
    platform = parse_repl(repl)
    assert len(platform.devices) == 1
