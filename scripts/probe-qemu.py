#!/usr/bin/env python3
"""
scripts/probe-qemu.py

A utility to extract the binary ground truth of C struct layouts and enums
directly from the compiled QEMU binary (qemu-system-arm).

It relies on `pahole` (from the 'dwarves' package) or falls back to `gdb`
to inspect the exact byte offsets and sizes of fields as determined by the C
compiler. This serves as the backend for the FFI validation suite.
"""

import argparse
import os
import subprocess
import sys
from pathlib import Path


def get_qemu_bin():
    # Priority:
    # 1. Environment variable
    # 2. Local build directory
    # 3. /opt/virtmcu install directory
    qemu_bin_env = os.environ.get("QEMU_BIN")
    if qemu_bin_env and Path(qemu_bin_env).exists():
        return qemu_bin_env

    build_dir = "build-virtmcu-asan" if os.environ.get("VIRTMCU_USE_ASAN") == "1" else "build-virtmcu"
    local_bin = Path("third_party/qemu") / build_dir / "qemu-system-arm"
    if local_bin.exists():
        return str(local_bin)

    local_bin_install = Path("third_party/qemu") / build_dir / "install/bin/qemu-system-arm"
    if local_bin_install.exists():
        return str(local_bin_install)

    opt_bin = Path("/opt/virtmcu/bin/qemu-system-arm")
    if opt_bin.exists():
        return str(opt_bin)

    return None


def probe_struct(qemu_bin, struct_name):
    """Probes a struct layout using pahole."""
    try:
        # pahole -C <struct_name> <binary>
        result = subprocess.run(
            ["pahole", "-C", struct_name, qemu_bin],
            capture_output=True,
            text=True,
            check=True,
        )
        return result.stdout
    except subprocess.CalledProcessError:
        # Fallback to gdb if pahole fails
        return probe_struct_gdb(qemu_bin, struct_name)


def probe_struct_gdb(qemu_bin, struct_name):
    """Fallback to gdb for probing struct layout."""
    gdb_cmd = f"ptype /o struct {struct_name}"
    try:
        result = subprocess.run(
            ["gdb", "-batch", "-ex", gdb_cmd, qemu_bin],
            capture_output=True,
            text=True,
            check=True,
        )
        return result.stdout
    except subprocess.CalledProcessError:
        return f"Error: Could not find struct {struct_name} in {qemu_bin}"


def probe_enum(qemu_bin, enum_name):
    """Probes an enum using gdb."""
    # Enums are harder with pahole, use gdb directly
    gdb_cmd = f"ptype enum {enum_name}"
    try:
        result = subprocess.run(
            ["gdb", "-batch", "-ex", gdb_cmd, qemu_bin],
            capture_output=True,
            text=True,
            check=True,
        )
        return result.stdout
    except subprocess.CalledProcessError:
        return f"Error: Could not find enum {enum_name} in {qemu_bin}"


def main():
    parser = argparse.ArgumentParser(description="Probe QEMU binary for struct layouts and enums.")
    parser.add_argument("name", help="Name of the struct or enum to probe")
    parser.add_argument(
        "--type",
        choices=["struct", "enum"],
        default="struct",
        help="Type of symbol (default: struct)",
    )
    parser.add_argument("--bin", help="Path to QEMU binary")

    args = parser.parse_args()

    qemu_bin = args.bin or get_qemu_bin()
    if not qemu_bin:
        print("Error: QEMU binary not found. Build QEMU or set QEMU_BIN.", file=sys.stderr)
        sys.exit(1)

    if args.type == "struct":
        print(probe_struct(qemu_bin, args.name))
    else:
        print(probe_enum(qemu_bin, args.name))


if __name__ == "__main__":
    main()
