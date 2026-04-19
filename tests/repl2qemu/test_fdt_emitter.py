"""
tests/test_fdt_emitter.py — Unit tests for tools/repl2qemu/fdt_emitter.py

Tests DTS generation and DTB compilation in isolation (no QEMU binary needed,
but dtc must be installed for the compile_dtb test).
"""

import os
import subprocess
import sys
from unittest.mock import patch

import pytest

# Import via the package so relative imports inside fdt_emitter resolve.
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "../../")))
from tools.repl2qemu.fdt_emitter import COMPAT_MAP, FdtEmitter, compile_dtb
from tools.repl2qemu.parser import ReplDevice, ReplInterrupt, ReplPlatform

# ── DTS structure ─────────────────────────────────────────────────────────────


def test_dts_header():
    emitter = FdtEmitter(ReplPlatform())
    dts = emitter.generate_dts()
    assert "/dts-v1/;" in dts
    assert "arm,generic-fdt" in dts
    assert "qemu:system-memory" in dts


def test_empty_platform_produces_valid_skeleton():
    """Even with no devices the DTS skeleton (cpus node, sysmem) must be present."""
    emitter = FdtEmitter(ReplPlatform())
    dts = emitter.generate_dts()
    assert "cpus {" in dts
    assert "qemu_sysmem" in dts


# ── CPU nodes ────────────────────────────────────────────────────────────────


def test_cpu_node_emitted():
    platform = ReplPlatform()
    platform.devices.append(
        ReplDevice(
            name="cpu0",
            type_name="CPU.ARMv7A",
            address_str="sysbus",
            properties={"cpuType": "cortex-a15"},
        )
    )
    dts = FdtEmitter(platform).generate_dts()
    assert "cortex-a15-arm-cpu" in dts
    assert 'device_type = "cpu"' in dts
    assert "cpu0@0" in dts


def test_multiple_cpu_nodes_indexed():
    platform = ReplPlatform()
    for i in range(2):
        platform.devices.append(
            ReplDevice(
                name=f"cpu{i}",
                type_name="CPU.ARMv7A",
                address_str="sysbus",
                properties={"cpuType": "cortex-a15"},
            )
        )
    dts = FdtEmitter(platform).generate_dts()
    assert "cpu0@0" in dts
    assert "cpu1@1" in dts


# ── Memory nodes ──────────────────────────────────────────────────────────────


def test_memory_node_emitted():
    platform = ReplPlatform()
    platform.devices.append(
        ReplDevice(
            name="sram",
            type_name="Memory.MappedMemory",
            address_str="0x20000000",
            properties={"size": "0x40000"},
        )
    )
    dts = FdtEmitter(platform).generate_dts()
    assert "memory@20000000" in dts
    assert "qemu-memory-region" in dts


# ── Peripheral nodes ──────────────────────────────────────────────────────────


def test_uart_pl011_node_emitted():
    platform = ReplPlatform()
    platform.devices.append(
        ReplDevice(
            name="uart0",
            type_name="UART.PL011",
            address_str="<0x09000000, +0x1000>",
            properties={},
        )
    )
    dts = FdtEmitter(platform).generate_dts()
    assert "pl011" in dts
    assert "uart0@9000000" in dts


def test_interrupt_emitted():
    platform = ReplPlatform()
    dev = ReplDevice(
        name="usart1",
        type_name="UART.STM32_UART",
        address_str="<0x40011000, +0x100>",
        properties={},
    )
    dev.interrupts.append(ReplInterrupt(source_range="0", target_device="nvic", target_range="37"))
    platform.devices.append(dev)
    dts = FdtEmitter(platform).generate_dts()
    assert "interrupts" in dts
    assert "37" in dts


def test_unknown_type_warns(capsys):
    platform = ReplPlatform()
    platform.devices.append(
        ReplDevice(
            name="mystery",
            type_name="Vendor.SomeUnknownPeripheral",
            address_str="0x10000000",
            properties={},
        )
    )
    FdtEmitter(platform).generate_dts()
    captured = capsys.readouterr()
    assert "Vendor.SomeUnknownPeripheral" in captured.err


def test_all_compat_map_types_produce_output():
    """Every type in COMPAT_MAP must produce a DTS node without crashing."""
    for renode_type, _ in COMPAT_MAP.items():
        if renode_type.startswith("CPU."):
            continue  # CPUs are handled separately
        platform = ReplPlatform()
        platform.devices.append(
            ReplDevice(
                name="dev",
                type_name=renode_type,
                address_str="<0x10000000, +0x1000>",
                properties={},
            )
        )
        dts = FdtEmitter(platform).generate_dts()
        assert COMPAT_MAP[renode_type] in dts, f"Expected '{COMPAT_MAP[renode_type]}' in DTS for type '{renode_type}'"


# ── DTB compilation ───────────────────────────────────────────────────────────


@pytest.mark.skipif(
    subprocess.run(["which", "dtc"], capture_output=True).returncode != 0,
    reason="dtc not installed",
)
def test_compile_dtb_produces_file(tmp_path):
    dts = """/dts-v1/;
/ {
    model = "test";
    compatible = "arm,generic-fdt";
    #address-cells = <1>;
    #size-cells = <1>;
};"""
    out = str(tmp_path / "test.dtb")
    result = compile_dtb(dts, out)
    assert result is True
    assert os.path.exists(out)
    assert os.path.getsize(out) > 0


@pytest.mark.skipif(
    subprocess.run(["which", "dtc"], capture_output=True).returncode != 0,
    reason="dtc not installed",
)
def test_compile_dtb_bad_dts_returns_false(tmp_path):
    out = str(tmp_path / "bad.dtb")
    result = compile_dtb("this is not valid DTS", out)
    assert result is False


def test_emitter_interrupt_parent():
    # Devices need to know who their interrupt parent is.
    # Currently fdt_emitter doesn't emit interrupt-parent properties.
    plat = ReplPlatform(
        devices=[
            ReplDevice(name="nvic", type_name="IRQControllers.NVIC", address_str="0xE000E100"),
            ReplDevice(
                name="uart1",
                type_name="UART.STM32_UART",
                address_str="0x40011000",
                interrupts=[ReplInterrupt("0", "nvic", "37")],
            ),
        ]
    )
    emitter = FdtEmitter(plat)
    dts = emitter.generate_dts()

    # We expect interrupt-parent to be set or at least the target to be a phandle
    # The current implementation just dumps <0 37 4> which is GIC format and assumes SPI
    assert "interrupt-parent" in dts or "interrupts-extended" in dts


def test_emitter_multiple_cpus():
    plat = ReplPlatform(
        devices=[
            ReplDevice(name="cpu0", type_name="CPU.CortexA", properties={"cpuType": "cortex-a15"}),
            ReplDevice(name="cpu1", type_name="CPU.CortexA", properties={"cpuType": "cortex-a15"}),
        ]
    )
    emitter = FdtEmitter(plat)
    dts = emitter.generate_dts()

    assert "cpu0@0" in dts
    assert "cpu1@1" in dts


def test_emitter_unmapped_device_warning(capsys):
    plat = ReplPlatform(devices=[ReplDevice(name="unknown", type_name="Unknown.Device", address_str="0x12345678")])
    emitter = FdtEmitter(plat)
    emitter.generate_dts()

    captured = capsys.readouterr()
    assert "Warning: no QEMU mapping for Renode type 'Unknown.Device'" in captured.err


def test_emitter_mem_int_size():
    plat = ReplPlatform(
        devices=[
            ReplDevice(name="ram", type_name="Memory.MappedMemory", address_str="0x80000000", properties={"size": 4096})
        ]
    )
    emitter = FdtEmitter(plat)
    dts = emitter.generate_dts()
    assert "reg = <0x0 0x80000000 0x0 0x1000>;" in dts


def test_emitter_boolean_props():
    plat = ReplPlatform(
        devices=[
            ReplDevice(
                name="dev",
                type_name="UART.PL011",
                address_str="0x40011000",
                properties={"prop_true": True, "prop_false": False},
            )
        ]
    )
    emitter = FdtEmitter(plat)
    dts = emitter.generate_dts()
    assert "prop_true;" in dts
    assert "prop_false" not in dts


def test_compile_dtb_failure():
    # Provide invalid DTS content
    with patch("subprocess.run", side_effect=subprocess.CalledProcessError(1, "dtc", stderr=b"syntax error")):
        success = compile_dtb("invalid dts", "/tmp/fail.dtb")
        assert success is False


def test_emitter_riscv():
    plat = ReplPlatform(
        devices=[
            ReplDevice(
                name="cpu",
                type_name="CPU.RISCV64",
                properties={"cpuType": "rv64", "isa": "rv64gc", "mmu-type": "riscv,sv39"},
            ),
            ReplDevice(
                name="ram", type_name="Memory.MappedMemory", address_str="0x80000000", properties={"size": "0x1000"}
            ),
        ]
    )
    emitter = FdtEmitter(plat)
    assert emitter.arch == "riscv"
    dts = emitter.generate_dts()
    assert "riscv,cpu-intc" in dts
    assert 'riscv,isa = "rv64gc"' in dts
    assert "timebase-frequency" in dts


def test_emitter_ranged_address():
    plat = ReplPlatform(devices=[ReplDevice(name="uart", type_name="UART.PL011", address_str="<0x40011000, +0x100>")])
    emitter = FdtEmitter(plat)
    dts = emitter.generate_dts()
    # base=0x40011000
    assert "uart@40011000" in dts


def test_emitter_invalid_address():
    plat = ReplPlatform(devices=[ReplDevice(name="uart", type_name="UART.PL011", address_str="not_an_address")])
    emitter = FdtEmitter(plat)
    dts = emitter.generate_dts()
    assert "uart@0" in dts


def test_emitter_gic_interrupts():
    plat = ReplPlatform(
        devices=[
            ReplDevice(name="gic", type_name="IRQControllers.ARM_GenericInterruptController", address_str="0x08000000"),
            ReplDevice(
                name="dev1",
                type_name="UART.PL011",
                address_str="0x09000000",
                interrupts=[
                    ReplInterrupt("0", "gic", "5"),  # < 32 -> should become 37
                    ReplInterrupt("1", "gic", "40"),  # >= 32 -> should stay 40
                ],
            ),
        ]
    )
    emitter = FdtEmitter(plat)
    dts = emitter.generate_dts()
    assert "interrupt-controller;" in dts
    assert "#interrupt-cells = <3>;" in dts
    assert "<0 37 4>" in dts
    assert "<0 40 4>" in dts


def test_emitter_invalid_hex_address():
    plat = ReplPlatform(devices=[ReplDevice(name="uart", type_name="UART.PL011", address_str="0x12G4")])
    emitter = FdtEmitter(plat)
    dts = emitter.generate_dts()
    assert "uart@0" in dts


def test_emitter_int_and_string_props():
    plat = ReplPlatform(
        devices=[
            ReplDevice(
                name="dev",
                type_name="UART.PL011",
                address_str="0x40011000",
                properties={"int_prop": 123, "str_prop": "hello"},
            )
        ]
    )
    emitter = FdtEmitter(plat)
    dts = emitter.generate_dts()
    assert "int_prop = <123>;" in dts
    assert 'str_prop = "hello";' in dts
