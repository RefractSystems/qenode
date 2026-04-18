import argparse
import sys

from .cli_generator import generate_cli
from .fdt_emitter import FdtEmitter, compile_dtb
from .parser import parse_repl


def main():
    parser = argparse.ArgumentParser(description="Convert Renode .repl to QEMU Device Tree")
    parser.add_argument("input", help="Path to .repl file")
    parser.add_argument("--out-dtb", help="Path to output .dtb file", required=True)
    parser.add_argument("--print-cmd", action="store_true", help="Print the recommended QEMU command")
    parser.add_argument("--out-arch", help="Path to output .arch file containing target architecture")

    args = parser.parse_args()

    try:
        with open(args.input, "r") as f:
            content = f.read()
    except FileNotFoundError:
        print(f"Error: File '{args.input}' not found.", file=sys.stderr)
        sys.exit(1)

    print(f"Parsing REPL: {args.input}...")
    platform = parse_repl(content)

    print(f"Generating Device Tree for {len(platform.devices)} devices...")
    emitter = FdtEmitter(platform)
    dts = emitter.generate_dts()

    if args.out_arch:
        with open(args.out_arch, "w") as f:
            f.write(emitter.arch)

    print(f"Compiling into '{args.out_dtb}'...")
    if compile_dtb(dts, args.out_dtb):
        print("✓ Success.")
    else:
        print("FAILED.")
        sys.exit(1)

    if args.print_cmd:
        cli_args, arch = generate_cli(platform, args.out_dtb)
        qemu_bin = "qemu-system-arm" if arch == "arm" else "qemu-system-riscv64"
        print("\nRecommended QEMU command:")
        print(f"{qemu_bin} {' '.join(cli_args)}")


if __name__ == "__main__":
    main()
