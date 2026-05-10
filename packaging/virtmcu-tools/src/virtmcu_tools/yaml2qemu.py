"""
Parses our modern YAML schema and returns (ReplPlatform, hints_dict).
hints_dict is reserved for future metadata (e.g. default clock rates); callers
that don't need it can unpack with ``platform, _ = parse_yaml_platform(path)``.
"""

from __future__ import annotations

import argparse
import logging
import os
import sys
from pathlib import Path

import fdt
import yaml

from .repl2qemu.fdt_emitter import FdtEmitter, compile_dtb
from .repl2qemu.parser import ReplDevice, ReplInterrupt, ReplPlatform
from .schema import World

# ==============================================================================
# yaml2qemu.py
#
# Parses the virtmcu YAML hardware description and translates it into a
# QEMU Device Tree (.dtb). This drives the FdtEmitter using the modern schema.
# ==============================================================================

logger = logging.getLogger(__name__)

# Basic logging configuration for standalone CLI usage
if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")


def get_node_ids(world: World) -> set[str]:
    """Returns a set of all valid node IDs in this world."""
    res = set()

    # 1. Try topology.nodes
    if world.topology and world.topology.nodes:
        for n in world.topology.nodes:
            res.add(str(n.name.root))
        return res

    # 2. Try top-level nodes
    if world.nodes:
        for n in world.nodes:
            res.add(str(n.name.root))
        return res

    # 3. Fallback to peripherals if they look like numeric IDs
    if world.peripherals:
        fallback_res = set()
        all_numeric = True
        for p in world.peripherals:
            name = str(p.name.root)
            if name.isdigit():
                fallback_res.add(name)
            else:
                all_numeric = False
                break

        if all_numeric and fallback_res:
            return fallback_res

    return res


def parse_yaml_platform(yaml_path: str | Path) -> tuple[ReplPlatform, dict[str, object]]:

    with Path(yaml_path).open() as f:
        content = f.read()

    try:
        world = World.model_validate(yaml.safe_load(content))
    except Exception as e:
        raise ValueError(f"Topology validation failed: {e}") from e

    has_topology_nodes = world.topology and world.topology.nodes
    has_toplevel_nodes = world.nodes

    if has_topology_nodes and has_toplevel_nodes:
        raise ValueError("Split-brain YAML detected")
    if world.peripherals:
        has_numeric_periphs = any(str(p.name.root).isdigit() for p in world.peripherals)
        if has_topology_nodes and has_numeric_periphs:
            raise ValueError("Split-brain YAML detected")

    valid_node_ids = get_node_ids(world)

    # Validate topology if present
    if world.topology:
        # Validate links
        if world.topology.links:
            for link in world.topology.links:
                for node_id in link.nodes:
                    if str(node_id.root) not in valid_node_ids:
                        raise ValueError(
                            f"Topology validation failed: node ID {node_id.root} in links not found in nodes"
                        )

        # Validate wireless
        if world.topology.wireless:
            for w_node in world.topology.wireless.nodes:
                if str(w_node.name.root) not in valid_node_ids:
                    raise ValueError(
                        f"Topology validation failed: node ID {w_node.name.root} in wireless nodes not found in nodes"
                    )

    platform = ReplPlatform()

    # 1. Map CPUs
    if world.machine and world.machine.cpus:
        for cpu in world.machine.cpus:
            cpu_type = cpu.type
            internal_type = "CPU.ARMv7A"
            if "riscv" in cpu_type.lower():
                internal_type = "CPU.RISCV64"

            dev = ReplDevice.create(
                name=cpu.name,
                type_name=internal_type,
                address_str="sysbus",
            )
            dev.properties["cpuType"] = cpu_type
            if internal_type == "CPU.RISCV64":
                if cpu.isa:
                    dev.properties["isa"] = cpu.isa
                if cpu.mmu_type:
                    dev.properties["mmu-type"] = cpu.mmu_type

            platform.devices.append(dev)

    # 2. Map Memory
    if world.memory:
        for m in world.memory:
            addr_val = m.address.root if m.address else 0
            address_str = hex(addr_val) if isinstance(addr_val, int) else str(addr_val)

            size = m.size.root if m.size else 0
            size_str = hex(size) if isinstance(size, int) else str(size)

            dev = ReplDevice.create(
                name=str(m.name.root),
                type_name="Memory.MappedMemory",
                address_str=address_str,
            )
            dev.properties["size"] = size_str
            platform.devices.append(dev)

    # 3. Map Peripherals
    if world.peripherals:
        for p in world.peripherals:
            # Support both 'renode_type' (for migrated files) or 'type' (for native ones)
            type_name = p.type or p.renode_type or "Unknown"

            addr_val = p.address.root if p.address else "none"
            address_str = hex(addr_val) if isinstance(addr_val, int) else str(addr_val)

            dev = ReplDevice.create(
                name=str(p.name.root),
                type_name=type_name,
                address_str=address_str,
                parent=p.parent,
            )
            dev.properties = p.properties.model_dump() if p.properties else {}

            # Parse interrupts if they exist
            if p.interrupts:
                for irq_entry in p.interrupts:
                    if isinstance(irq_entry, int):
                        # Native YAML format: just the IRQ number
                        dev.interrupts.append(ReplInterrupt("0", "none", str(irq_entry)))
                    elif isinstance(irq_entry, str) and "@" in irq_entry:
                        # Legacy repl2yaml format: target@line
                        target, line = irq_entry.split("@")
                        dev.interrupts.append(ReplInterrupt("0", target, line))

            # SVD Augmentation
            if "svd" in dev.properties:
                try:
                    from cmsis_svd.model import SVDPeripheral
                    from cmsis_svd.parser import SVDParser

                    svd_path_raw = dev.properties["svd"]
                    if not isinstance(svd_path_raw, str):
                        raise TypeError(f"svd property must be a string, got {type(svd_path_raw)}")

                    svd_path: str = str(svd_path_raw)

                    # Provide an absolute path if necessary based on some workspace context, or assume it's relative to current dir
                    if not Path(svd_path).exists() and "VIRTMCU_WORKSPACE" in os.environ:
                        svd_path = str(Path(os.environ["VIRTMCU_WORKSPACE"]) / svd_path)

                    if Path(svd_path).exists():
                        parser = SVDParser.for_xml_file(svd_path)
                        svd_device = parser.get_device()
                        # Assuming the first peripheral in SVD corresponds to this device
                        if svd_device.peripherals:
                            first_periph = svd_device.peripherals[0]
                            if not isinstance(first_periph, SVDPeripheral):
                                raise TypeError("Expected SVDPeripheral")

                            # Update address if it was not explicitly provided or was "none"
                            if address_str == "none" and first_periph.base_address is not None:
                                address_str = hex(first_periph.base_address)
                                dev.address_str = address_str

                            # If size is missing, try to infer it
                            if "size" not in dev.properties and "region-size" not in dev.properties:
                                if first_periph.address_blocks and first_periph.address_blocks[0].size is not None:
                                    dev.properties["size"] = hex(first_periph.address_blocks[0].size)
                                else:
                                    dev.properties["size"] = "0x1000"  # Fallback
                except (FileNotFoundError, ValueError, ImportError, TypeError) as e:
                    logger.error(f"Could not parse SVD file {dev.properties['svd']} for {dev.name}: {e}")

            platform.devices.append(dev)

    return platform, {}


def validate_dtb(dtb_path: str | Path, devices: list[ReplDevice]) -> None:
    """
    Task 2: Validate DTB contains all expected peripherals.
    Uses the fdt library to parse the DTB directly for structured validation.
    """
    try:
        with Path(dtb_path).open("rb") as f:
            dtb = fdt.parse_dtb(f.read())

        missing = []

        def find_node_recursive(parent: fdt.Node, prefix: str) -> fdt.Node | None:
            """Recursively finds a node that matches the prefix exactly or matches prefix@address."""
            for node in parent.nodes:
                if node.name == prefix or node.name.startswith(prefix + "@"):
                    return node
                # Recurse into children
                found = find_node_recursive(node, prefix)
                if found:
                    return found
            return None

        for dev in devices:
            if "CPU" in dev.type_name:
                # CPUs are typically under /cpus/
                cpus_node = dtb.root.get_subnode("cpus")
                if not cpus_node:
                    logger.error("ERROR: No 'cpus' node found in DTB!")
                    missing.append("cpus")
                    continue

                cpu_node = find_node_recursive(cpus_node, dev.name)
                if cpu_node:
                    if not cpu_node.get_property("memory"):
                        logger.error(f"ERROR: CPU node '{dev.name}' is missing 'memory' property!")
                        missing.append(f"{dev.name} (missing memory binding)")
                else:
                    missing.append(dev.name)
                continue

            if dev.type_name == "chardev":
                continue  # CLI-only, no DTB node

            # Check for name or name@address
            if dev.type_name == "Memory.MappedMemory":
                if "@" in dev.name:
                    prefix = f"memory@{dev.name.split('@')[1]}"
                elif getattr(dev, "address_str", None) and isinstance(dev.address_str, str):
                    addr_hex = hex(int(dev.address_str, 0)).replace("0x", "")
                    prefix = f"memory@{addr_hex}"
                else:
                    prefix = "memory"
            else:
                prefix = dev.name

            dev_node = find_node_recursive(dtb.root, prefix)

            if not dev_node:
                missing.append(dev.name)
            elif dev.type_name == "Memory.MappedMemory" and "size" in dev.properties:
                # Verify memory size matches
                reg_prop = dev_node.get_property("reg")
                if reg_prop:
                    try:
                        # VirtMCU uses #address-cells = 2, #size-cells = 2
                        # reg = <base_hi base_lo size_hi size_lo>
                        cells = reg_prop.data
                        if len(cells) >= 4:
                            size_hi = cells[2]
                            size_lo = cells[3]
                            actual_size = (size_hi << 32) | size_lo

                            size_val = dev.properties["size"]
                            if isinstance(size_val, str):
                                expected_size = int(size_val, 0)
                            elif isinstance(size_val, int):
                                expected_size = size_val
                            else:
                                raise TypeError(f"Invalid size property type: {type(size_val)}")

                            if actual_size != expected_size:
                                logger.error(
                                    f"ERROR: Memory node '{dev.name}' size mismatch! Expected {hex(expected_size)}, found {hex(actual_size)}"
                                )
                                missing.append(f"{dev.name} (size mismatch)")
                        else:
                            logger.error(f"Memory node '{dev.name}' has unexpected reg property length: {len(cells)}")
                    except (ValueError, TypeError, IndexError) as e:
                        logger.error(f"⚠️ Warning: Could not verify size for {dev.name}: {e}")
                else:
                    logger.error(f"ERROR: Memory node '{dev.name}' is missing 'reg' property!")
                    missing.append(f"{dev.name} (missing reg)")

        if missing:
            logger.error(
                f"ERROR: The following peripherals from YAML are missing in the generated DTB: {', '.join(missing)}"
            )
            logger.error("This usually means the device type is unknown to FdtEmitter or the address mapping failed.")
            logger.error("FAILED: DTB validation failed.")
            sys.exit(1)
        logger.info("✓ Validation successful.")
    except Exception as e:
        logger.error(f"ERROR: Could not validate DTB: {e}")
        sys.exit(1)


def main() -> None:
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

    # Extract transport from world
    world_data = yaml.safe_load(Path(args.input).read_text())
    world = World.model_validate(world_data)
    transport_val = "zenoh"
    if world.topology and world.topology.transport:
        transport_val = world.topology.transport.value

    # Inject Transport Hub
    hub_name = "hub0"
    env_router = os.environ.get("VIRTMCU_ZENOH_ROUTER")
    hub_dev = ReplDevice.create(
        name=hub_name,
        type_name="Virtmcu.TransportHub",
        address_str="none",
    )
    if env_router:
        hub_dev.properties["router"] = env_router

    platform.devices.insert(0, hub_dev)

    original_devices = list(platform.devices)

    # Extract architecture
    emitter = FdtEmitter(platform)
    arch = emitter.arch
    if args.out_arch:
        with Path(args.out_arch).open("w") as f:
            f.write(arch)

    # Extract devices that require explicit CLI instantiation.
    cli_args = []
    filtered_devices = []
    for dev in platform.devices:
        if dev.type_name == "mmio-socket-bridge" and "socket-path" not in dev.properties:
            logger.error("Missing mandatory property: socket-path")
            sys.exit(1)

        # Link all virtmcu peripherals to the hub
        is_virtmcu_native = dev.type_name in (
            "telemetry",
            "ieee802154",
            "zenoh-wifi",
            "wifi",
            "spi",
            "spi-echo",
            "canfd",
            "flexray",
            "lin",
            "clock",
            "ui",
            "actuator",
            "s32k144-lpuart",
        )
        if is_virtmcu_native:
            dev.properties["transport"] = hub_name

        if dev.type_name == "chardev":
            node = dev.properties.get("node", "0")
            router = dev.properties.get("router") or env_router
            topic = dev.properties.get("topic")
            chardev_id = dev.properties.get("id", f"chr_{dev.name}")

            # chardev is not yet migrated to Hub QOM links, so we keep string transport for now.
            chardev_arg = f"virtmcu,id={chardev_id},node={node},transport={transport_val}"
            if router:
                chardev_arg += f",router={router}"
            if topic:
                chardev_arg += f",topic={topic}"

            cli_args.append("-chardev")
            cli_args.append(chardev_arg)
            if str(dev.properties.get("serial", "true")).lower() == "true":
                cli_args.append("-serial")
                cli_args.append(f"chardev:{chardev_id}")
        elif dev.type_name in ("telemetry", "ieee802154"):
            # These are now handled via DTB
            filtered_devices.append(dev)
        elif dev.type_name == "mmio-socket-bridge":
            # Handled via DTB (both memory map and instantiation) but we also need a -device CLI arg.
            filtered_devices.append(dev)

            # Auto-generate the CLI arguments for mmio-socket-bridge
            base_addr = dev.address_str
            region_size = dev.properties.get("region-size", dev.properties.get("size", "0x1000"))
            socket_path = dev.properties.get("socket-path", "")

            # If base_addr is hex string, keep it, else maybe fallback to properties
            if not base_addr or base_addr == "none":
                base_addr = str(dev.properties["base-addr"]) if "base-addr" in dev.properties else "0"

            bridge_id = dev.properties.get("id", dev.name)

            dev_arg = f"mmio-socket-bridge,id={bridge_id},base-addr={base_addr},region-size={region_size},socket-path={socket_path}"

            if "reconnect-ms" in dev.properties:
                dev_arg += f",reconnect-ms={dev.properties['reconnect-ms']}"

            cli_args.append("-device")
            cli_args.append(dev_arg)
        else:
            if dev.type_name == "zenoh-wifi":
                dev.type_name = "wifi"
            filtered_devices.append(dev)

    platform.devices = filtered_devices

    logger.info(f"Generating Device Tree for {len(platform.devices)} devices...")
    dts = emitter.generate_dts()

    if args.out_cli:
        with Path(args.out_cli).open("w") as f:
            f.writelines(arg + "\n" for arg in cli_args)

    logger.info(f"Compiling into '{args.out_dtb}'...")
    if compile_dtb(dts, args.out_dtb):
        logger.info("✓ Compilation Success.")
        validate_dtb(args.out_dtb, original_devices)
    else:
        logger.error("FAILED.")
        sys.exit(1)


if __name__ == "__main__":
    main()
