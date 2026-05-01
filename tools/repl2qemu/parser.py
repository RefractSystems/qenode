"""Factory method to create strongly-typed devices based on type_name."""

from __future__ import annotations

import logging
import re
from dataclasses import dataclass, field
from pathlib import Path

logger = logging.getLogger(__name__)


@dataclass
class ReplInterrupt:
    source_range: str  # "0" or "0-3"
    target_device: str
    target_range: str  # "37" or "19-22"


@dataclass
class ReplDevice:
    name: str
    type_name: str
    address_str: str | None = None
    parent: str | None = None
    properties: dict[str, object] = field(default_factory=dict)
    interrupts: list[ReplInterrupt] = field(default_factory=list)

    @classmethod
    def create(
        cls,
        name: str,
        type_name: str,
        address_str: str | None = None,
        parent: str | None = None,
        properties: dict[str, object] | None = None,
        interrupts: list[ReplInterrupt] | None = None,
    ) -> ReplDevice:

        props = properties if properties is not None else {}
        irqs = interrupts if interrupts is not None else []

        if type_name in ("IRQControllers.GIC", "IRQControllers.ARM_GenericInterruptController"):
            return GicDevice(
                name=name,
                type_name=type_name,
                address_str=address_str,
                parent=parent,
                properties=props,
                interrupts=irqs,
            )

        if type_name == "IRQControllers.NVIC":
            return NvicDevice(
                name=name,
                type_name=type_name,
                address_str=address_str,
                parent=parent,
                properties=props,
                interrupts=irqs,
            )

        if type_name == "Memory.MappedMemory":
            return MemoryDevice(
                name=name,
                type_name=type_name,
                address_str=address_str,
                parent=parent,
                properties=props,
                interrupts=irqs,
            )

        if type_name == "mmio-socket-bridge":
            return MmioBridgeDevice(
                name=name,
                type_name=type_name,
                address_str=address_str,
                parent=parent,
                properties=props,
                interrupts=irqs,
            )

        if type_name in ("ieee802154", "telemetry"):
            return WirelessDevice(
                name=name,
                type_name=type_name,
                address_str=address_str,
                parent=parent,
                properties=props,
                interrupts=irqs,
            )

        return ReplDevice(
            name=name,
            type_name=type_name,
            address_str=address_str,
            parent=parent,
            properties=props,
            interrupts=irqs,
        )


@dataclass
class GicDevice(ReplDevice):
    interrupt_cells: int = 3


@dataclass
class NvicDevice(ReplDevice):
    interrupt_cells: int = 1


@dataclass
class MemoryDevice(ReplDevice):
    pass  # we'll migrate size parsing here


@dataclass
class MmioBridgeDevice(ReplDevice):
    pass


@dataclass
class WirelessDevice(ReplDevice):
    pass


@dataclass
class ReplPlatform:
    devices: list[ReplDevice] = field(default_factory=list)


def parse_repl(content: str, base_dir: str = ".") -> ReplPlatform:
    """
    Parses a Renode .repl file and extracts device definitions, ignoring complex
    inline initialization blocks and non-device nodes.
    """
    platform = ReplPlatform()
    current_device: ReplDevice | None = None

    # Regex patterns
    # Device header: usart1: UART.STM32_UART @ sysbus <0x40011000, +0x100>
    re_device = re.compile(r"^([a-zA-Z0-9_]+):\s+([a-zA-Z0-9_.]+)(?:\s+@\s+(sysbus\s+)?([^{]*)?)?(?:\s+@\s*\{)?")
    # Property: size: 0x10000000
    re_prop = re.compile(r"^\s+([a-zA-Z0-9_]+):\s+(.+)$")
    # Interrupt (simple): -> nvic@37 or ->gic@1
    re_irq_simple = re.compile(r"^\s+->\s*([a-zA-Z0-9_]+)@(\d+)$")
    # Interrupt (ranged): [0-3] -> nvic@[19-22]
    re_irq_range = re.compile(r"^\s+\[(\d+-\d+)\]\s+->\s+([a-zA-Z0-9_]+)@\[(\d+-\d+)\]$")
    # Using: using "path/to/file.repl"
    re_using = re.compile(r'^using\s+"([^"]+)"')

    in_multiline_block = False

    for raw_line in content.splitlines():
        # Remove comments and strip trailing whitespace
        line = raw_line.split("//")[0].rstrip()
        if not line:
            continue

        # Handle 'using' statement
        match_using = re_using.match(line)
        if match_using:
            included_file = match_using.group(1)
            full_path = Path(base_dir) / included_file
            if Path(full_path).exists():
                with Path(full_path).open() as f:
                    included_platform = parse_repl(f.read(), str(Path(full_path).parent))
                    platform.devices.extend(included_platform.devices)
            else:
                logger.warning(f"Warning: Included file not found: {full_path}")
            continue

        # Start of a new device block (no leading whitespace)
        if not line.startswith(" ") and not line.startswith("\t") and ":" in line:
            match = re_device.match(line)
            if match:
                name, type_name, _, addr = match.groups()
                # Clean up address string (e.g., remove trailing { if inline block)
                if addr:
                    addr = addr.strip().split("{")[0].strip()
                    if addr.endswith("@"):
                        addr = addr[:-1].strip()
                    if addr.lower() == "sysbus":
                        addr = None

                current_device = ReplDevice.create(name, type_name, addr)
                platform.devices.append(current_device)
            else:
                # Could be a generic sysbus block or tag block, ignore
                current_device = None

            # If the device header ALSO contains the start of a block, mark it
            if "{" in line and "}" not in line:
                in_multiline_block = True

            continue

        # Handle multi-line blocks like { ... } that aren't on the device header line
        if "{" in line and "}" not in line and not in_multiline_block:
            in_multiline_block = True
            continue

        if in_multiline_block:
            if "}" in line:
                in_multiline_block = False

            # If there's an address inside the block, extract it
            # e.g., sysbus new BusMultiRegistration { address: 0x8000000; ... }
            if current_device and "address:" in line:
                match = re.search(r"address:\s*(0x[0-9a-fA-F]+)", line)
                if match and not current_device.address_str:
                    current_device.address_str = match.group(1)
            continue

        # If we are inside a device block, parse properties and interrupts
        if current_device:
            # Try simple interrupt: -> nvic@37
            match = re_irq_simple.match(line)
            if match:
                target, irq_line = match.groups()
                current_device.interrupts.append(ReplInterrupt("0", target, irq_line))
                continue

            # Try ranged interrupt: [0-3] -> nvic@[19-22]
            match = re_irq_range.match(line)
            if match:
                src_range, target, irq_range = match.groups()
                current_device.interrupts.append(ReplInterrupt(src_range, target, irq_range))
                continue

            # Try property: size: 0x10000000
            match = re_prop.match(line)
            if match:
                key, val = match.groups()
                val = val.strip()
                # Strip quotes from string values
                if (val.startswith('"') and val.endswith('"')) or (val.startswith("'") and val.endswith("'")):
                    val = val[1:-1]
                current_device.properties[key] = val
                continue

    return platform


if __name__ == "__main__":
    import sys

    logging.basicConfig(level=logging.INFO, stream=sys.stdout)
    filename = sys.argv[1] if len(sys.argv) > 1 else "third_party/renode/platforms/boards/cortex_a53_virtio.repl"
    with Path(filename).open() as f:
        plat = parse_repl(f.read())
        for dev in plat.devices:
            logger.info(f"{dev.name} ({dev.type_name}) @ {dev.address_str}")
            for k, v in dev.properties.items():
                logger.info(f"  {k}: {v}")
            for irq in dev.interrupts:
                logger.info(f"  IRQ {irq.source_range} -> {irq.target_device}@{irq.target_range}")
