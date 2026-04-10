import subprocess
from dataclasses import dataclass
from typing import List, Dict, Optional
import os

from .parser import ReplPlatform

def generate_cli(platform: ReplPlatform, dtb_path: str) -> List[str]:
    """Generates the QEMU CLI arguments based on the parsed platform."""
    args = [
        "-M", f"arm-generic-fdt,hw-dtb={dtb_path}",
        "-nographic",
    ]
    
    cpu_type = None
    for dev in platform.devices:
        if dev.type_name == "CPU.CortexM":
            cpu_type = "m"
        elif dev.type_name == "CPU.CortexA":
            cpu_type = "a"
            
    # As per ADR-009, if it's Cortex-M, force TCG. If Cortex-A and on Linux, use KVM/TCG.
    if cpu_type == "m":
        args.extend(["-accel", "tcg"])
    else:
        # Default to TCG for now, but in a real scenario we might sniff the host OS
        args.extend(["-accel", "tcg"])
        
    return args

if __name__ == "__main__":
    from parser import parse_repl
    with open("third_party/renode/platforms/cpus/stm32f4.repl", "r") as f:
        plat = parse_repl(f.read())
    cli = generate_cli(plat, "test.dtb")
    print("qemu-system-arm " + " ".join(cli))
