import os
import sys

sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "../../tools/repl2qemu")))
from parser import parse_repl


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
