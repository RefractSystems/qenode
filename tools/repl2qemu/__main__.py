from __future__ import annotations

import argparse
import logging
import sys
from pathlib import Path

from tools.repl2qemu.cli_generator import generate_cli
from tools.repl2qemu.fdt_emitter import FdtEmitter, compile_dtb
from tools.repl2qemu.parser import parse_repl

logger = logging.getLogger(__name__)


def main() -> None:
    parser = argparse.ArgumentParser(description="Convert Renode .repl to QEMU Device Tree")
    parser.add_argument("input", help="Path to .repl file")
    parser.add_argument("--out-dtb", help="Path to output .dtb file", required=True)
    parser.add_argument("--print-cmd", action="store_true", help="Print the recommended QEMU command")
    parser.add_argument("--out-arch", help="Path to output .arch file containing target architecture")

    args = parser.parse_args()

    try:
        with Path(args.input).open() as f:
            content = f.read()
    except FileNotFoundError:
        logger.error(f"Error: File '{args.input}' not found.")
        sys.exit(1)

    logger.info(f"Parsing REPL: {args.input}...")
    platform = parse_repl(content)

    logger.info(f"Generating Device Tree for {len(platform.devices)} devices...")
    emitter = FdtEmitter(platform)
    dts = emitter.generate_dts()

    if args.out_arch:
        with Path(args.out_arch).open("w") as f:
            f.write(emitter.arch)

    logger.info(f"Compiling into '{args.out_dtb}'...")
    if compile_dtb(dts, args.out_dtb):
        logger.info("✓ Success.")
    else:
        logger.error("FAILED.")
        sys.exit(1)

    if args.print_cmd:
        cli_args, arch = generate_cli(platform, args.out_dtb)
        qemu_bin = "qemu-system-arm" if arch == "arm" else "qemu-system-riscv64"
        logger.info("\nRecommended QEMU command:")
        logger.info(f"{qemu_bin} {' '.join(cli_args)}")


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    main()
