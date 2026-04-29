#!/usr/bin/env python3
# ==============================================================================
# repl2yaml.py
#
# Migrates legacy Renode .repl hardware descriptions to the modern virtmcu YAML
# format. This ensures that we can transition to an OpenUSD-aligned schema
# without losing existing board definitions.
# ==============================================================================

import argparse
import logging
from pathlib import Path
from typing import Any

import yaml

from .repl2qemu.parser import parse_repl

logger = logging.getLogger(__name__)


def migrate(repl_path: str, yaml_path: str):
    logger.info(f"Reading Renode platform: {repl_path}")
    with Path(repl_path).open() as f:
        plat = parse_repl(f.read())

    # Build the YAML structure
    # We try to infer a sensible machine name from the filename
    machine_name = Path(repl_path).stem

    output: dict[str, Any] = {
        "machine": {"name": machine_name, "type": "arm-generic-fdt", "cpus": []},
        "peripherals": [],
    }

    for dev in plat.devices:
        # Separate CPUs from Peripherals for better hierarchical structure
        if "CPU" in dev.type_name:
            cpu_info = {"name": dev.name, "type": dev.properties.get("cpuType", "cortex-a15"), "memory": "sysmem"}
            output["machine"]["cpus"].append(cpu_info)
            continue

        # Normal peripheral
        p: dict[str, Any] = {
            "name": dev.name,
            "renode_type": dev.type_name,
            "address": dev.address_str,
        }

        # Add properties if they exist
        if dev.properties:
            p["properties"] = dev.properties

        # Add interrupts
        if dev.interrupts:
            # For simplicity in this migration, we store the raw target string
            p["interrupts"] = [f"{irq.target_device}@{irq.target_range}" for irq in dev.interrupts]

        # Standard virtmcu requirement: everything connects to sysmem
        p["container"] = "sysmem"

        output["peripherals"].append(p)

    logger.info(f"Writing virtmcu YAML: {yaml_path}")
    with Path(yaml_path).open("w") as f:
        yaml.dump(output, f, sort_keys=False, default_flow_style=False)


def main():
    parser = argparse.ArgumentParser(description="Convert Renode .repl to virtmcu YAML")
    parser.add_argument("input", help="Path to .repl file")
    parser.add_argument("--out", help="Path to output .yaml file (default: same name)")

    args = parser.parse_args()

    out_path = args.out if args.out else Path(args.input).stem + ".yaml"
    migrate(args.input, out_path)


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    main()
