#!/usr/bin/env python3
# ==============================================================================
# yaml2qemu.py
#
# Parses the virtmcu YAML hardware description and translates it into a 
# QEMU Device Tree (.dtb). This drives the FdtEmitter using the modern schema.
# ==============================================================================

import os
import sys
import yaml
import argparse

from .repl2qemu.parser import ReplPlatform, ReplDevice, ReplInterrupt
from .repl2qemu.fdt_emitter import FdtEmitter, compile_dtb

def parse_yaml_platform(yaml_path: str) -> ReplPlatform:
    """
    Parses our modern YAML schema and returns a ReplPlatform AST.
    """
    with open(yaml_path, "r") as f:
        data = yaml.safe_load(f)

    platform = ReplPlatform()
    
    # 1. Map CPUs
    for cpu in data.get("machine", {}).get("cpus", []):
        dev = ReplDevice(
            name=cpu["name"],
            type_name="CPU.ARMv7A", # Use supported internal type
            address_str="sysbus",
            properties={"cpuType": cpu["type"]}
        )
        platform.devices.append(dev)

    # 2. Map Peripherals
    for p in data.get("peripherals", []):
        # Support both 'renode_type' (for migrated files) or 'type' (for native ones)
        type_name = p.get("type") or p.get("renode_type", "Unknown")
        
        dev = ReplDevice(
            name=p["name"],
            type_name=type_name,
            address_str=str(p.get("address", "none")),
            properties=p.get("properties", {})
        )
        
        # Parse interrupts if they exist
        for irq_str in p.get("interrupts", []):
            if "@" in irq_str:
                target, line = irq_str.split("@")
                dev.interrupts.append(ReplInterrupt("0", target, line))
        
        platform.devices.append(dev)

    return platform

def main():
    parser = argparse.ArgumentParser(description="Convert virtmcu YAML to Device Tree")
    parser.add_argument("input", help="Path to .yaml file")
    parser.add_argument("--out-dtb", help="Path to output .dtb file", required=True)
    
    args = parser.parse_args()
    
    if not os.path.exists(args.input):
        print(f"Error: Input file '{args.input}' not found.")
        sys.exit(1)
        
    print(f"Parsing YAML: {args.input}...")
    platform = parse_yaml_platform(args.input)
    
    print(f"Generating Device Tree for {len(platform.devices)} devices...")
    emitter = FdtEmitter(platform)
    dts = emitter.generate_dts()
    
    print(f"Compiling into '{args.out_dtb}'...")
    if compile_dtb(dts, args.out_dtb):
        print("✓ Success.")
    else:
        print("FAILED.")
        sys.exit(1)

if __name__ == "__main__":
    main()
