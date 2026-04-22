#!/usr/bin/env python3
"""
scripts/check-ffi.py

The "FFI Gate" verification tool.
This script compares Rust struct FFI assertions (size_of, offset_of) against
the actual ground-truth layouts inside the compiled QEMU binary.

It uses `scripts/probe-qemu.py` (which wraps `pahole` or `gdb`) to extract
the binary offsets, then parses the corresponding Rust source files to ensure
they match. Run with `--fix` to automatically synchronize the Rust code.
"""

import argparse
import re
import subprocess
import sys
from pathlib import Path

# List of structs to verify
STRUCTS_TO_CHECK = [
    "ObjectClass",
    "ChardevClass",
    "Chardev",
    "ChardevZenohOptions",
    "Netdev",
    "NetClientState",
    "NetClientInfo",
]


def parse_pahole(output):
    """Parses pahole output into a dict of {field_name: offset}."""
    fields = {}
    # Match lines like: /*    112      |       8 */    char *label;
    # Or: /*      0      |      40 */    Object parent_obj;
    # Or: /*    152      |       8 */    unsigned long features[1];
    # Or total size: /* total size (bytes):  160 */

    field_re = re.compile(r"/\*\s+(\d+)\s+\|\s+(\d+)\s+\*/\s+(?:struct\s+)?[\w\* ]+\s*([\w\[\]]+);")
    size_re = re.compile(r"/\*\s+total size \(bytes\):\s+(\d+)\s+\*/")

    for line in output.splitlines():
        field_match = field_re.search(line)
        if field_match:
            offset = int(field_match.group(1))
            name = field_match.group(3).split("[")[0]  # handle features[1]
            fields[name] = offset

        size_match = size_re.search(line)
        if size_match:
            fields["__size__"] = int(size_match.group(1))

    return fields


def check_rust_file(file_path, probed_layouts, fix=False):
    """Checks and optionally fixes FFI assertions in a Rust file."""
    p = Path(file_path)
    if not p.exists():
        return True, 0

    content = p.read_text()

    new_content = content
    success = True
    fixes_count = 0

    # Matches: assert!(core::mem::offset_of!(Chardev, label) == 112);
    offset_re = re.compile(r"assert!\(core::mem::offset_of!\((\w+),\s*(\w+)\)\s*==\s*(\d+)\);")
    # Matches: assert!(core::mem::size_of::<Chardev>() == 160);
    size_re = re.compile(r"assert!\(core::mem::size_of::<(\w+)>\(\)\s*==\s*(\d+)\);")

    def offset_sub(m):
        nonlocal success, fixes_count
        struct_name = m.group(1)
        field_name = m.group(2)
        current_offset = int(m.group(3))

        if struct_name in probed_layouts:
            probed = probed_layouts[struct_name]
            if field_name in probed:
                actual_offset = probed[field_name]
                if actual_offset != current_offset:
                    print(
                        f"Mismatch in {struct_name}.{field_name}: Rust expects {current_offset}, binary has {actual_offset}"
                    )
                    success = False
                    if fix:
                        fixes_count += 1
                        return f"assert!(core::mem::offset_of!({struct_name}, {field_name}) == {actual_offset});"
        return m.group(0)

    def size_sub(m):
        nonlocal success, fixes_count
        struct_name = m.group(1)
        current_size = int(m.group(2))

        if struct_name in probed_layouts:
            probed = probed_layouts[struct_name]
            if "__size__" in probed:
                actual_size = probed["__size__"]
                if actual_size != current_size:
                    print(f"Mismatch in {struct_name} size: Rust expects {current_size}, binary has {actual_size}")
                    success = False
                    if fix:
                        fixes_count += 1
                        return f"assert!(core::mem::size_of::<{struct_name}>() == {actual_size});"
        return m.group(0)

    new_content = offset_re.sub(offset_sub, new_content)
    new_content = size_re.sub(size_sub, new_content)

    if fix and fixes_count > 0:
        p.write_text(new_content)
        print(f"Applied {fixes_count} fixes to {file_path}")
        return True, fixes_count

    return success, fixes_count


def main():
    parser = argparse.ArgumentParser(description="Check Rust FFI layouts against QEMU binary.")
    parser.add_argument("--fix", action="store_true", help="Automatically fix mismatches in Rust code")
    parser.add_argument("--bin", help="Path to QEMU binary")
    args = parser.parse_args()

    probe_script = str(Path(__file__).parent / "probe-qemu.py")
    qemu_bin = args.bin

    probed_layouts = {}
    print("==> Probing QEMU binary for struct layouts...")
    for struct in STRUCTS_TO_CHECK:
        cmd = [probe_script, struct]
        if qemu_bin:
            cmd.extend(["--bin", qemu_bin])

        result = subprocess.run(cmd, capture_output=True, text=True, check=False)
        if result.returncode == 0:
            probed_layouts[struct] = parse_pahole(result.stdout)
        else:
            print(f"Warning: Could not probe struct {struct}")

    rust_files = [
        "hw/rust/virtmcu-qom/src/qom.rs",
        "hw/rust/virtmcu-qom/src/chardev.rs",
        "hw/rust/virtmcu-qom/src/net.rs",
        "hw/rust/zenoh-chardev/src/lib.rs",
        "hw/rust/zenoh-netdev/src/lib.rs",
    ]

    overall_success = True
    total_fixes = 0
    for f in rust_files:
        success, fixes = check_rust_file(f, probed_layouts, fix=args.fix)
        overall_success &= success
        total_fixes += fixes

    if not overall_success and not args.fix:
        print("\n❌ FFI layout mismatch detected! Run './scripts/check-ffi.py --fix' to sync.")
        sys.exit(1)

    if total_fixes > 0:
        print(f"\n✅ Synced {total_fixes} FFI layout assertions.")
    else:
        print("\n✅ FFI layouts are in sync.")


if __name__ == "__main__":
    main()
