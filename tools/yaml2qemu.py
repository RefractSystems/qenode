#!/usr/bin/env python3
# ==============================================================================
# yaml2qemu.py
#
# Parses the virtmcu YAML hardware description and translates it into a
# QEMU Device Tree (.dtb). This drives the FdtEmitter using the modern schema.
# ==============================================================================

import argparse
import logging
import os
import subprocess
import sys
from pathlib import Path

import yaml

from .repl2qemu.fdt_emitter import FdtEmitter, compile_dtb
from .repl2qemu.parser import ReplDevice, ReplInterrupt, ReplPlatform

logger = logging.getLogger(__name__)

# Basic logging configuration for standalone CLI usage
if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")


def parse_yaml_platform(yaml_path: str) -> tuple[ReplPlatform, dict]:
    """
    Parses our modern YAML schema and returns (ReplPlatform, hints_dict).
    hints_dict is reserved for future metadata (e.g. default clock rates); callers
    that don't need it can unpack with ``platform, _ = parse_yaml_platform(path)``.
    """
    with Path(yaml_path).open() as f:
        data = yaml.safe_load(f)

    # Hardening: Check for unknown top-level keys to prevent silent failures
    KNOWN_KEYS = {"machine", "peripherals", "memory", "include", "nodes", "topology"}  # noqa: N806
    for key in data:
        if key not in KNOWN_KEYS:
            logger.warning(f"Warning: Unknown top-level key '{key}' in {yaml_path}. It will be ignored.")

    # Validate topology if present
    topology = data.get("topology")
    if topology:
        nodes = data.get("nodes", [])
        valid_node_ids = set()

        # If nodes is a list of dicts, extract 'id' or 'name'. If it's a list of ints/strings, use them.
        for node in nodes:
            if isinstance(node, dict) and "id" in node:
                valid_node_ids.add(str(node["id"]))
            elif isinstance(node, (int, str)):
                valid_node_ids.add(str(node))

        # Validate global_seed
        seed = topology.get("global_seed", 0)
        if not isinstance(seed, int) or seed < 0:
            raise ValueError(f"Topology validation failed: global_seed must be a non-negative integer, got {seed}")

        # Validate max_messages_per_node_per_quantum
        max_msgs = topology.get("max_messages_per_node_per_quantum", 1024)
        if not isinstance(max_msgs, int) or max_msgs < 1:
            raise ValueError(
                f"Topology validation failed: max_messages_per_node_per_quantum must be a positive integer, got {max_msgs}"
            )

        # Validate links
        for link in topology.get("links", []):
            for node_id in link.get("nodes", []):
                if str(node_id) not in valid_node_ids:
                    raise ValueError(f"Topology validation failed: node ID {node_id} in links not found in nodes:")

        # Validate wireless
        wireless = topology.get("wireless", {})
        for w_node in wireless.get("nodes", []):
            node_id = w_node.get("id")
            if str(node_id) not in valid_node_ids:
                raise ValueError(f"Topology validation failed: node ID {node_id} in wireless nodes not found in nodes:")

    platform = ReplPlatform()

    # 1. Map CPUs
    for cpu in data.get("machine", {}).get("cpus", []):
        cpu_type = cpu["type"]
        internal_type = "CPU.ARMv7A"
        if "riscv" in cpu_type.lower():
            internal_type = "CPU.RISCV64"

        dev = ReplDevice.create(
            name=cpu["name"],
            type_name=internal_type,
            address_str="sysbus",
        )
        dev.properties["cpuType"] = cpu_type
        if internal_type == "CPU.RISCV64":
            if "isa" in cpu:
                dev.properties["isa"] = cpu["isa"]
            if "mmu-type" in cpu:
                dev.properties["mmu-type"] = cpu["mmu-type"]

        platform.devices.append(dev)

    # 2. Map Memory
    # Support a dedicated 'memory' section to avoid requiring 'Memory.MappedMemory' type in peripherals.
    for m in data.get("memory", []):
        addr_val = m.get("address", 0)
        address_str = hex(addr_val) if isinstance(addr_val, int) else str(addr_val)

        # properties must be dict[str, str] for ReplDevice, but we support ints in YAML
        size = m.get("size", 0)
        size_str = hex(size) if isinstance(size, int) else str(size)

        dev = ReplDevice.create(
            name=m["name"],
            type_name="Memory.MappedMemory",
            address_str=address_str,
        )
        dev.properties["size"] = size_str
        platform.devices.append(dev)

    # 3. Map Peripherals
    for p in data.get("peripherals", []):
        # Support both 'renode_type' (for migrated files) or 'type' (for native ones)
        type_name = p.get("type") or p.get("renode_type", "Unknown")

        addr_val = p.get("address", "none")
        address_str = hex(addr_val) if isinstance(addr_val, int) else str(addr_val)

        dev = ReplDevice.create(
            name=p["name"],
            type_name=type_name,
            address_str=address_str,
            parent=p.get("parent"),
        )
        dev.properties = p.get("properties", {})

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
            if dev.type_name == "chardev":
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

            if dts_node not in dts and dev.name not in dts:
                missing.append(dev.name)
            elif (
                (dts_node in dts or dev.name in dts)
                and dev.type_name == "Memory.MappedMemory"
                and "size" in dev.properties
            ):
                # Task: Verify memory size matches
                try:
                    target_node = dts_node if dts_node in dts else dev.name
                    size_val = dev.properties["size"]
                    expected_size = int(size_val, 0) if isinstance(size_val, str) else int(size_val)
                    # Simple heuristic: find the node block and check reg property
                    node_start = dts.find(target_node)
                    node_end = dts.find("};", node_start)
                    node_content = dts[node_start:node_end]

                    import re

                    # Robust regex: handle 0x or decimal, and varying whitespace
                    reg_match = re.search(
                        r"reg = <((?:0x)?[0-9a-fA-F]+)\s+((?:0x)?[0-9a-fA-F]+)\s+((?:0x)?[0-9a-fA-F]+)\s+((?:0x)?[0-9a-fA-F]+)>",
                        node_content,
                    )
                    if reg_match:

                        def to_int(s):
                            return int(s, 16) if s.startswith("0x") else int(s)

                        size_hi = to_int(reg_match.group(3))
                        size_lo = to_int(reg_match.group(4))
                        actual_size = (size_hi << 32) | size_lo
                        if actual_size != expected_size:
                            logger.error(
                                f"ERROR: Memory node '{dev.name}' size mismatch! Expected {hex(expected_size)}, found {hex(actual_size)}"
                            )
                            missing.append(f"{dev.name} (size mismatch)")
                except Exception as e:
                    logger.warning(f"⚠️ Warning: Could not verify size for {dev.name}: {e}")

        if missing:
            logger.error(
                f"ERROR: The following peripherals from YAML are missing in the generated DTB: {', '.join(missing)}"
            )
            logger.error("This usually means the device type is unknown to FdtEmitter or the address mapping failed.")
            logger.error("FAILED: DTB validation failed.")
            sys.exit(1)
        logger.info("✓ Validation successful.")
    except subprocess.CalledProcessError as e:
        logger.warning(f"⚠️ Warning: dtc failed during validation: {e.stderr}")
    except FileNotFoundError:
        logger.error(
            "ERROR: 'dtc' (device-tree-compiler) not found — DTB validation skipped. "
            "Install dtc to enable post-build validation."
        )
        sys.exit(1)
    except Exception as e:
        logger.warning(f"⚠️ Warning: Could not validate DTB: {e}")


def main():
    parser = argparse.ArgumentParser(description="Convert virtmcu YAML to Device Tree")
    parser.add_argument("input", help="Path to .yaml file")
    parser.add_argument("--out-dtb", help="Path to output .dtb file", required=True)
    parser.add_argument("--out-cli", help="Path to output .cli file for extra arguments")
    parser.add_argument("--out-arch", help="Path to output .arch file containing target architecture")

    args = parser.parse_args()

    if not Path(args.input).exists():
        logger.error(f"Error: Input file '{args.input}' not found.")
        sys.exit(1)

    logger.info(f"Parsing YAML: {args.input}...")
    try:
        platform, _ = parse_yaml_platform(args.input)
    except ValueError as e:
        logger.error(f"ERROR: {e}")
        sys.exit(1)

    original_devices = list(platform.devices)

    # Extract architecture
    emitter = FdtEmitter(platform)
    arch = emitter.arch
    if args.out_arch:
        with Path(args.out_arch).open("w") as f:
            f.write(arch)

    # Extract transport from topology
    topology: dict[str, str] = {}  # topology is parsed in coordinator instead
    transport = topology.get("transport", "zenoh")

    # Extract devices that require explicit CLI instantiation.
    # chardev: CLI-only (no DTB node).
    # telemetry: Handled via DTB + CLI-only side effects (not anymore, DTB only now).
    # mmio-socket-bridge: Handled via DTB (both memory map and instantiation).
    cli_args = []
    filtered_devices = []
    env_router = os.environ.get("VIRTMCU_ZENOH_ROUTER")
    for dev in platform.devices:
        if dev.type_name == "mmio-socket-bridge" and "socket-path" not in dev.properties:
            logger.error("Missing mandatory property: socket-path")
            sys.exit(1)
        if dev.type_name == "chardev":
            node = dev.properties.get("node", "0")
            router = dev.properties.get("router") or env_router
            topic = dev.properties.get("topic")
            chardev_id = dev.properties.get("id", f"chr_{dev.name}")

            chardev_arg = f"virtmcu,id={chardev_id},node={node},transport={transport}"
            if router:
                chardev_arg += f",router={router}"
            if topic:
                chardev_arg += f",topic={topic}"

            cli_args.append("-chardev")
            cli_args.append(chardev_arg)
            cli_args.append("-serial")
            cli_args.append(f"chardev:{chardev_id}")
        elif dev.type_name in ("telemetry", "ieee802154"):
            # These are now handled via DTB but need transport hint
            dev.properties["transport"] = transport
            if env_router and "router" not in dev.properties:
                dev.properties["router"] = env_router
            filtered_devices.append(dev)
        elif dev.type_name == "mmio-socket-bridge":
            # Handled via DTB (both memory map and instantiation).
            filtered_devices.append(dev)
        else:
            if dev.type_name == "zenoh-wifi":
                dev.type_name = "wifi"
            filtered_devices.append(dev)

    platform.devices = filtered_devices

    logger.info(f"Generating Device Tree for {len(platform.devices)} devices...")
    dts = emitter.generate_dts()

    if args.out_cli:
        with Path(args.out_cli).open("w") as f:
            for arg in cli_args:
                f.write(arg + "\n")

    logger.info(f"Compiling into '{args.out_dtb}'...")
    if compile_dtb(dts, args.out_dtb):
        logger.info("✓ Compilation Success.")
        validate_dtb(args.out_dtb, original_devices)
    else:
        logger.error("FAILED.")
        sys.exit(1)


if __name__ == "__main__":
    main()
