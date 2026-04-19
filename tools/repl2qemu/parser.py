import os
import re
from dataclasses import dataclass, field
from typing import Dict, List, Optional


@dataclass
class ReplInterrupt:
    source_range: str  # "0" or "0-3"
    target_device: str
    target_range: str  # "37" or "19-22"


@dataclass
class ReplDevice:
    name: str
    type_name: str
    address_str: Optional[str] = None
    parent: Optional[str] = None
    properties: Dict[str, str] = field(default_factory=dict)
    interrupts: List[ReplInterrupt] = field(default_factory=list)


@dataclass
class ReplPlatform:
    devices: List[ReplDevice] = field(default_factory=list)


def parse_repl(content: str, base_dir: str = ".") -> ReplPlatform:
    """
    Parses a Renode .repl file and extracts device definitions, ignoring complex
    inline initialization blocks and non-device nodes.
    """
    platform = ReplPlatform()
    current_device: Optional[ReplDevice] = None

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
            full_path = os.path.join(base_dir, included_file)
            if os.path.exists(full_path):
                with open(full_path, "r") as f:
                    included_platform = parse_repl(f.read(), os.path.dirname(full_path))
                    platform.devices.extend(included_platform.devices)
            else:
                print(f"Warning: Included file not found: {full_path}")
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

                current_device = ReplDevice(name, type_name, addr)
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

    filename = sys.argv[1] if len(sys.argv) > 1 else "third_party/renode/platforms/boards/cortex_a53_virtio.repl"
    with open(filename, "r") as f:
        plat = parse_repl(f.read())
        for dev in plat.devices:
            print(f"{dev.name} ({dev.type_name}) @ {dev.address_str}")
            for k, v in dev.properties.items():
                print(f"  {k}: {v}")
            for irq in dev.interrupts:
                print(f"  IRQ {irq.source_range} -> {irq.target_device}@{irq.target_range}")
