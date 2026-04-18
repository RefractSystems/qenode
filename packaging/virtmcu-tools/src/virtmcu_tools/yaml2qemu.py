#!/usr/bin/env python3
# ==============================================================================
# yaml2qemu.py
#
# Parses the virtmcu YAML hardware description and translates it into a
# QEMU Device Tree (.dtb). This drives the FdtEmitter using the modern schema.
# ==============================================================================

import argparse
import os
import subprocess
import sys

import yaml

from .repl2qemu.fdt_emitter import FdtEmitter, compile_dtb
from .repl2qemu.parser import ReplDevice, ReplInterrupt, ReplPlatform


def parse_yaml_platform(yaml_path: str) -> tuple[ReplPlatform, dict]:
    """
    Parses our modern YAML schema and returns (ReplPlatform, hints_dict).
    hints_dict is reserved for future metadata (e.g. default clock rates); callers
    that don't need it can unpack with ``platform, _ = parse_yaml_platform(path)``.
    """
    with open(yaml_path, "r") as f:
        data = yaml.safe_load(f)

    platform = ReplPlatform()

    # 1. Map CPUs
    for cpu in data.get("machine", {}).get("cpus", []):
        cpu_type = cpu["type"]
        internal_type = "CPU.ARMv7A"
        if "riscv" in cpu_type.lower():
            internal_type = "CPU.RISCV64"

        dev = ReplDevice(
            name=cpu["name"],
            type_name=internal_type,
            address_str="sysbus",
            properties={"cpuType": cpu_type},
        )
        if internal_type == "CPU.RISCV64":
            if "isa" in cpu:
                dev.properties["isa"] = cpu["isa"]
            if "mmu-type" in cpu:
                dev.properties["mmu-type"] = cpu["mmu-type"]

        platform.devices.append(dev)

    # 2. Map Peripherals
    for p in data.get("peripherals", []):
        # Support both 'renode_type' (for migrated files) or 'type' (for native ones)
        type_name = p.get("type") or p.get("renode_type", "Unknown")

        addr_val = p.get("address", "none")
        address_str = hex(addr_val) if isinstance(addr_val, int) else str(addr_val)

        dev = ReplDevice(
            name=p["name"], type_name=type_name, address_str=address_str, properties=p.get("properties", {})
        )

        # Parse interrupts if they exist
        for irq_entry in p.get("interrupts", []):
            if isinstance(irq_entry, int):
                # Native YAML format: just the IRQ number
                dev.interrupts.append(ReplInterrupt("0", "none", str(irq_entry)))
            elif isinstance(irq_entry, str) and "@" in irq_entry:
                # Legacy repl2yaml format: target@line
                target, line = irq_entry.split("@")
                dev.interrupts.append(ReplInterrupt("0", target, line))

        platform.devices.append(dev)

    return platform, {}


def validate_dtb(dtb_path, devices):
    """
    Task 2: Validate DTB contains all expected peripherals.
    Decompiles the DTB back to DTS and ensures each peripheral is present.
    """
    try:
        res = subprocess.run(["dtc", "-I", "dtb", "-O", "dts", dtb_path], capture_output=True, text=True, check=True)
        dts = res.stdout

        missing = []
        for dev in devices:
            if "CPU" in dev.type_name:
                continue
            if dev.type_name in ("zenoh-chardev", "zenoh-telemetry"):
                continue  # CLI-only, no DTB node

            # Check for name@address (DTS node format), e.g. "uart0@9000000".
            # Memory nodes are special: FdtEmitter always names them "memory@..."
            try:
                addr_int = int(dev.address_str, 0)
                if dev.type_name == "Memory.MappedMemory":
                    dts_node = f"memory@{addr_int:x}"
                else:
                    dts_node = f"{dev.name}@{addr_int:x}"
            except (ValueError, TypeError):
                dts_node = dev.name  # fallback for non-numeric address strings

            if dts_node not in dts:
                missing.append(dev.name)

        if missing:
            print(
                f"ERROR: The following peripherals from YAML are missing in the generated DTB: {', '.join(missing)}",
                file=sys.stderr,
            )
            print(
                "This usually means the device type is unknown to FdtEmitter or the address mapping failed.",
                file=sys.stderr,
            )
            print("FAILED: DTB validation failed.")
            sys.exit(1)
        print("✓ Validation successful.")
    except subprocess.CalledProcessError as e:
        print(f"⚠️ Warning: dtc failed during validation: {e.stderr}", file=sys.stderr)
    except FileNotFoundError:
        print(
            "ERROR: 'dtc' (device-tree-compiler) not found — DTB validation skipped. "
            "Install dtc to enable post-build validation.",
            file=sys.stderr,
        )
        sys.exit(1)
    except Exception as e:
        print(f"⚠️ Warning: Could not validate DTB: {e}", file=sys.stderr)


def main():
    parser = argparse.ArgumentParser(description="Convert virtmcu YAML to Device Tree")
    parser.add_argument("input", help="Path to .yaml file")
    parser.add_argument("--out-dtb", help="Path to output .dtb file", required=True)
    parser.add_argument("--out-cli", help="Path to output .cli file for extra arguments")
    parser.add_argument("--out-arch", help="Path to output .arch file containing target architecture")

    args = parser.parse_args()

    if not os.path.exists(args.input):
        print(f"Error: Input file '{args.input}' not found.")
        sys.exit(1)

    print(f"Parsing YAML: {args.input}...")
    platform, _ = parse_yaml_platform(args.input)
    original_devices = list(platform.devices)

    # Extract architecture
    emitter = FdtEmitter(platform)
    arch = emitter.arch
    if args.out_arch:
        with open(args.out_arch, "w") as f:
            f.write(arch)

    # Extract devices that require explicit CLI instantiation.
    # zenoh-chardev: CLI-only (no DTB node).
    # zenoh-telemetry: CLI-only (no DTB node).
    # mmio-socket-bridge: Handled via DTB (both memory map and instantiation).
    cli_args = []
    filtered_devices = []
    for dev in platform.devices:
        if dev.type_name == "zenoh-chardev":
            node = dev.properties.get("node", "0")
            router = dev.properties.get("router")
            topic = dev.properties.get("topic")
            chardev_id = dev.properties.get("id", f"chr_{dev.name}")

            chardev_arg = f"zenoh,id={chardev_id},node={node}"
            if router:
                chardev_arg += f",router={router}"
            if topic:
                chardev_arg += f",topic={topic}"

            cli_args.append("-chardev")
            cli_args.append(chardev_arg)
            cli_args.append("-serial")
            cli_args.append(f"chardev:{chardev_id}")
        elif dev.type_name == "zenoh-telemetry":
            node = dev.properties.get("node", "0")
            router = dev.properties.get("router")
            device_arg = f"zenoh-telemetry,node={node}"
            if router:
                device_arg += f",router={router}"
            cli_args.append("-device")
            cli_args.append(device_arg)
        elif dev.type_name == "zenoh-802154":
            node = dev.properties.get("node", "0")
            router = dev.properties.get("router")
            topic = dev.properties.get("topic")
            device_arg = f"zenoh-802154,node={node}"
            if router:
                device_arg += f",router={router}"
            if topic:
                device_arg += f",topic={topic}"
            cli_args.append("-device")
            cli_args.append(device_arg)
            filtered_devices.append(dev)  # Keep in DTB
        else:
            filtered_devices.append(dev)

    platform.devices = filtered_devices

    print(f"Generating Device Tree for {len(platform.devices)} devices...")
    dts = emitter.generate_dts()

    if args.out_cli:
        with open(args.out_cli, "w") as f:
            for arg in cli_args:
                f.write(arg + "\n")

    print(f"Compiling into '{args.out_dtb}'...")
    if compile_dtb(dts, args.out_dtb):
        print("✓ Compilation Success.")
        validate_dtb(args.out_dtb, original_devices)
    else:
        print("FAILED.")
        sys.exit(1)


if __name__ == "__main__":
    main()
