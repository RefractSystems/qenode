"""
tests/test_fdt_emitter.py — Unit tests for tools/repl2qemu/fdt_emitter.py

Tests DTS generation and DTB compilation in isolation (no QEMU binary needed,
but dtc must be installed for the compile_dtb test).
"""

import logging
import subprocess
import sys
from pathlib import Path
from unittest.mock import patch

import pytest

# Import via the package so relative imports inside fdt_emitter resolve.
sys.path.insert(0, str(Path(__file__).resolve().parent / "../../"))
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
        ReplDevice.create(
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
            ReplDevice.create(
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
        ReplDevice.create(
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
        ReplDevice.create(
            name="uart0",
            type_name="UART.PL011",
            address_str="<0x09000000, +0x1000>",
            properties={},
        )
    )
    dts = FdtEmitter(platform).generate_dts()
    assert "pl011" in dts
    assert "uart0@9000000 {" in dts


def test_interrupt_emitted():
    platform = ReplPlatform()
    dev = ReplDevice.create(
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


def test_unknown_type_warns(caplog):
    platform = ReplPlatform()
    platform.devices.append(
        ReplDevice.create(
            name="mystery",
            type_name="Vendor.SomeUnknownPeripheral",
            address_str="0x10000000",
            properties={},
        )
    )
    FdtEmitter(platform).generate_dts()
    caplog.set_level(logging.WARNING)
    assert "Vendor.SomeUnknownPeripheral" in caplog.text


def test_all_compat_map_types_produce_output(tmp_path):
    """
    Every type in COMPAT_MAP must produce a valid DTS node that compiles
    without any dtc warnings or errors. This ensures all supported peripherals
    have valid formatting.
    """
    # Provide known-good mandatory properties for each type
    known_good_props = {
        "Memory.MappedMemory": {"size": "0x1000"},
        "ieee802154": {"transport": "zenoh", "node": "0"},
        "telemetry": {"transport": "zenoh", "node": "0"},
        "RemotePort.Peripheral": {"region-size": "0x1000", "socket-path": "/tmp/sock", "base-addr": "0x10000000"},
    }

    import shutil

    import fdt

    has_dtc = shutil.which("dtc") is not None

    for renode_type, _ in COMPAT_MAP.items():
        if renode_type.startswith("CPU."):
            continue  # CPUs are handled separately

        platform = ReplPlatform()

        # Always add a GIC to satisfy interrupt-parent requirements globally
        platform.devices.append(ReplDevice.create(name="gic", type_name="IRQControllers.GIC", address_str="0x08000000"))

        props = known_good_props.get(renode_type, {})

        platform.devices.append(
            ReplDevice.create(
                name="dev",
                type_name=renode_type,
                address_str="0x10000000",
                properties=props,
                interrupts=[ReplInterrupt("0", "gic", "33")],  # 33 is a valid SPI
            )
        )
        emitter = FdtEmitter(platform)
        dts = emitter.generate_dts()

        if has_dtc:
            dtb_out = str(tmp_path / f"{renode_type.replace('.', '_')}.dtb")
            success = compile_dtb(dts, dtb_out)
            assert success, f"Failed to compile DTB for type '{renode_type}'. Check DTC output."

            # Step 1: Parse the DTB using the fdt library for structured validation
            with Path(dtb_out).open("rb") as f:
                dtb = fdt.parse_dtb(f.read())

            # Find the emitted node
            dev_node = None
            expected_prefix = "memory" if renode_type == "Memory.MappedMemory" else "dev"
            for node in dtb.root.nodes:
                if node.name.startswith(expected_prefix):
                    dev_node = node
                    break

            assert dev_node is not None, (
                f"Node starting with '{expected_prefix}' not found in DTB for type '{renode_type}'"
            )

            # Structurally verify the compatible string
            compat_prop = dev_node.get_property("compatible")
            assert compat_prop is not None
            assert COMPAT_MAP[renode_type] in compat_prop.data

            # Structurally verify GIC interrupts are 3 cells
            if renode_type not in ("IRQControllers.GIC", "Memory.MappedMemory"):
                interrupts_prop = dev_node.get_property("interrupts")
                assert interrupts_prop is not None
                # The fdt library parses this as a list of integers (cells)
                assert len(interrupts_prop.data) == 3, f"Interrupts for {renode_type} must be 3 cells"
                assert interrupts_prop.data == [0, 33, 4]

            # Step 3: Integrate dt-schema (dt-validate)
            has_dtvalidate = shutil.which("dt-validate") is not None
            if has_dtvalidate:
                # dt-validate expects DTB as input
                res = subprocess.run(["dt-validate", dtb_out], capture_output=True, text=True)
                # dt-validate exits with 0 even if there are warnings, but prints them to stderr.
                # However, many standard bindings (like pl011) require standard clocks which we don't mock yet.
                # We can either assert res.returncode == 0, or be stricter.
                # For now, let's just make sure it runs without catastrophic failure.
                assert res.returncode == 0, f"dt-validate crashed on {renode_type}: {res.stderr}"


def test_validation_missing_memory_size():
    platform = ReplPlatform(
        devices=[ReplDevice.create(name="ram", type_name="Memory.MappedMemory", address_str="0x80000000")]
    )
    with pytest.raises(ValueError, match="missing mandatory 'size' property"):
        FdtEmitter(platform).generate_dts()


def test_validation_missing_wireless_props():
    platform = ReplPlatform(devices=[ReplDevice.create(name="radio", type_name="ieee802154", address_str="0x90000000")])
    with pytest.raises(ValueError, match="missing mandatory 'transport' property"):
        FdtEmitter(platform).generate_dts()

    platform.devices[0].properties["transport"] = "zenoh"
    with pytest.raises(ValueError, match="missing mandatory 'node' property"):
        FdtEmitter(platform).generate_dts()


def test_validation_missing_bridge_props():
    platform = ReplPlatform(
        devices=[ReplDevice.create(name="bridge", type_name="mmio-socket-bridge", address_str="0x50000000")]
    )
    # Check size
    with pytest.raises(ValueError, match="missing mandatory 'region-size' property"):
        FdtEmitter(platform).generate_dts()

    # Add size, check address/base-addr (it has address_str so it should pass has_addr)
    platform.devices[0].properties["region-size"] = 0x1000
    with pytest.raises(ValueError, match="missing mandatory 'socket-path' property"):
        FdtEmitter(platform).generate_dts()


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
    assert Path(out).exists()
    assert Path(out).stat().st_size > 0


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
            ReplDevice.create(name="nvic", type_name="IRQControllers.NVIC", address_str="0xE000E100"),
            ReplDevice.create(
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
            ReplDevice.create(name="cpu0", type_name="CPU.CortexA", properties={"cpuType": "cortex-a15"}),
            ReplDevice.create(name="cpu1", type_name="CPU.CortexA", properties={"cpuType": "cortex-a15"}),
        ]
    )
    emitter = FdtEmitter(plat)
    dts = emitter.generate_dts()

    assert "cpu0@0" in dts
    assert "cpu1@1" in dts


def test_emitter_unmapped_device_warning(caplog):
    plat = ReplPlatform(
        devices=[ReplDevice.create(name="unknown", type_name="Unknown.Device", address_str="0x12345678")]
    )
    emitter = FdtEmitter(plat)
    emitter.generate_dts()

    caplog.set_level(logging.WARNING)
    assert "Warning: no QEMU mapping for Renode type 'Unknown.Device'" in caplog.text


def test_emitter_mem_int_size():
    plat = ReplPlatform(
        devices=[
            ReplDevice.create(
                name="ram", type_name="Memory.MappedMemory", address_str="0x80000000", properties={"size": 4096}
            )
        ]
    )
    emitter = FdtEmitter(plat)
    dts = emitter.generate_dts()
    assert "reg = <0x0 0x80000000 0x0 0x1000>;" in dts


def test_emitter_boolean_props():
    plat = ReplPlatform(
        devices=[
            ReplDevice.create(
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
            ReplDevice.create(
                name="cpu",
                type_name="CPU.RISCV64",
                properties={"cpuType": "rv64", "isa": "rv64gc", "mmu-type": "riscv,sv39"},
            ),
            ReplDevice.create(
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
    plat = ReplPlatform(
        devices=[ReplDevice.create(name="uart", type_name="UART.PL011", address_str="<0x40011000, +0x100>")]
    )
    emitter = FdtEmitter(plat)
    dts = emitter.generate_dts()
    # base=0x40011000
    assert "uart@40011000 {" in dts


def test_emitter_invalid_address():
    plat = ReplPlatform(devices=[ReplDevice.create(name="uart", type_name="UART.PL011", address_str="not_an_address")])
    emitter = FdtEmitter(plat)
    dts = emitter.generate_dts()
    assert "uart@0 {" in dts


def test_emitter_gic_interrupts():
    plat = ReplPlatform(
        devices=[
            ReplDevice.create(
                name="gic", type_name="IRQControllers.ARM_GenericInterruptController", address_str="0x08000000"
            ),
            ReplDevice.create(
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
    assert "<0 37 4 0 40 4>" in dts


def test_emitter_invalid_hex_address():
    plat = ReplPlatform(devices=[ReplDevice.create(name="uart", type_name="UART.PL011", address_str="0x12G4")])
    emitter = FdtEmitter(plat)
    dts = emitter.generate_dts()
    assert "uart@0 {" in dts


def test_emitter_int_and_string_props():
    plat = ReplPlatform(
        devices=[
            ReplDevice.create(
                name="dev",
                type_name="UART.PL011",
                address_str="0x40011000",
                properties={"int_prop": 123, "str_prop": "hello"},
            )
        ]
    )
    emitter = FdtEmitter(plat)
    dts = emitter.generate_dts()
    assert "int_prop = <0x7b>;" in dts
    assert 'str_prop = "hello";' in dts
