import argparse
import os
import sys

from .cli_generator import generate_cli
from .fdt_emitter import FdtEmitter, compile_dtb
from .parser import parse_repl


def main():
    parser = argparse.ArgumentParser(description="repl2qemu: Translate Renode .repl files to QEMU Device Trees")
    parser.add_argument("input_file", help="Path to the input .repl file")
    parser.add_argument("--out-dtb", help="Path to the output .dtb file", required=True)
    parser.add_argument("--print-cmd", action="store_true", help="Print the equivalent QEMU CLI command")

    args = parser.parse_args()

    if not os.path.exists(args.input_file):
        print(f"Error: Input file '{args.input_file}' not found.")
        sys.exit(1)

    with open(args.input_file, "r") as f:
        content = f.read()

    print(f"Parsing '{args.input_file}'...")
    platform = parse_repl(content)

    print(f"Generating DTS for {len(platform.devices)} parsed devices...")
    emitter = FdtEmitter(platform)
    dts = emitter.generate_dts()

    print(f"Compiling DTS into '{args.out_dtb}'...")
    if not compile_dtb(dts, args.out_dtb):
        print("Compilation failed.")
        sys.exit(1)

    if args.print_cmd:
        cli = generate_cli(platform, args.out_dtb)
        print("\nQEMU Command:")
        print("qemu-system-arm " + " ".join(cli))


if __name__ == "__main__":
    main()
