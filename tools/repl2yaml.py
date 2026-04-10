#!/usr/bin/env python3
# ==============================================================================
# repl2yaml.py
#
# Migrates legacy Renode .repl hardware descriptions to the modern virtmcu YAML 
# format. This ensures that we can transition to an OpenUSD-aligned schema 
# without losing existing board definitions.
# ==============================================================================

import os
import sys
import yaml
import argparse

from .repl2qemu.parser import parse_repl

def migrate(repl_path: str, yaml_path: str):
    print(f"Reading Renode platform: {repl_path}")
    with open(repl_path, "r") as f:
        plat = parse_repl(f.read())

    # Build the YAML structure
    # We try to infer a sensible machine name from the filename
    machine_name = os.path.splitext(os.path.basename(repl_path))[0]
    
    output = {
        "machine": {
            "name": machine_name,
            "type": "arm-generic-fdt",
            "cpus": []
        },
        "peripherals": []
    }

    for dev in plat.devices:
        # Separate CPUs from Peripherals for better hierarchical structure
        if "CPU" in dev.type_name:
            cpu_info = {
                "name": dev.name,
                "type": dev.properties.get("cpuType", "cortex-a15"),
                "memory": "sysmem"
            }
            output["machine"]["cpus"].append(cpu_info)
            continue

        # Normal peripheral
        p = {
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

    print(f"Writing virtmcu YAML: {yaml_path}")
    with open(yaml_path, "w") as f:
        yaml.dump(output, f, sort_keys=False, default_flow_style=False)

def main():
    parser = argparse.ArgumentParser(description="Convert Renode .repl to virtmcu YAML")
    parser.add_argument("input", help="Path to .repl file")
    parser.add_argument("--out", help="Path to output .yaml file (default: same name)")
    
    args = parser.parse_args()
    
    out_path = args.out if args.out else os.path.splitext(args.input)[0] + ".yaml"
    migrate(args.input, out_path)

if __name__ == "__main__":
    main()
