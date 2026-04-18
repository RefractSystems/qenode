from typing import List, Tuple

from .parser import ReplPlatform


def generate_cli(platform: ReplPlatform, dtb_path: str) -> Tuple[List[str], str]:
    """
    Generates the QEMU CLI arguments based on the parsed platform.
    Returns (args_list, arch_string).
    """
    arch = "arm"
    cpu_type_m = False
    for dev in platform.devices:
        if "RISCV" in dev.type_name.upper():
            arch = "riscv"
            break
        if dev.type_name == "CPU.CortexM":
            cpu_type_m = True

    if arch == "riscv":
        args = [
            "-M",
            "virt",
            "-dtb",
            dtb_path,
            "-nographic",
            "-bios",
            "none",
        ]
    else:
        args = [
            "-M",
            f"arm-generic-fdt,hw-dtb={dtb_path}",
            "-nographic",
        ]

    # As per ADR-009, if it's Cortex-M, force TCG. If Cortex-A and on Linux, use KVM/TCG.
    if cpu_type_m:
        args.extend(["-accel", "tcg"])
    else:
        # Default to TCG for now
        args.extend(["-accel", "tcg"])

    return args, arch
