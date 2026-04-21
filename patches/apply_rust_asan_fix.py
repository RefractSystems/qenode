#!/usr/bin/env python3
import re
import sys
from pathlib import Path


def main():
    if len(sys.argv) < 2:
        print("Usage: apply_rust_asan_fix.py <qemu_dir>")
        sys.exit(1)

    qemu_dir = Path(sys.argv[1])
    meson_build = qemu_dir / "meson.build"

    if not meson_build.exists():
        print(f"  -> {meson_build} not found, skipping Rust ASan patch")
        sys.exit(0)

    content = meson_build.read_text()
    changed = False

    # 1. Patch AddressSanitizer (asan)
    # Target: qemu_ldflags = ['-fsanitize=address'] + qemu_ldflags
    # Injection:
    #     if have_rust
    #       add_project_arguments('-C', 'link-arg=-fsanitize=address', language: 'rust')
    #     endif

    asan_injection = "add_project_arguments('-C', 'link-arg=-fsanitize=address', language: 'rust')"
    if asan_injection not in content:
        # Match the line regardless of indentation
        pattern = r"(\s+)(qemu_ldflags\s+=\s+\['-fsanitize=address'\]\s+\+\s+qemu_ldflags)"
        match = re.search(pattern, content)
        if match:
            indent = match.group(1)
            target_line = match.group(0)
            insertion = f"\n{indent}if have_rust\n{indent}  {asan_injection}\n{indent}endif"
            content = content.replace(target_line, target_line + insertion)
            print("  -> Added ASan flags for Rust in meson.build")
            changed = True

    # 2. Patch UndefinedBehaviorSanitizer (ubsan)
    # Target: qemu_ldflags += ['-fsanitize=undefined']
    # Injection:
    #     if have_rust
    #       add_project_arguments('-C', 'link-arg=-fsanitize=undefined', language: 'rust')
    #     endif

    ubsan_injection = "add_project_arguments('-C', 'link-arg=-fsanitize=undefined', language: 'rust')"
    if ubsan_injection not in content:
        pattern = r"(\s+)(qemu_ldflags\s+=\s+\['-fsanitize=undefined'\]|qemu_ldflags\s+=\s+qemu_ldflags\s+\+\s+\['-fsanitize=undefined'\]|qemu_ldflags\s+\+=\s+\['-fsanitize=undefined'\])"
        match = re.search(pattern, content)
        if match:
            indent = match.group(1)
            target_line = match.group(0)
            insertion = f"\n{indent}if have_rust\n{indent}  {ubsan_injection}\n{indent}endif"
            content = content.replace(target_line, target_line + insertion)
            print("  -> Added UBSan flags for Rust in meson.build")
            changed = True

    if changed:
        meson_build.write_text(content)
        print("✓ Patched meson.build for Rust ASan/UBSan support")
    else:
        # Check if they were already present (to avoid confusing output)
        if asan_injection in content and ubsan_injection in content:
            print("  -> Rust ASan/UBSan support already present in meson.build")
        else:
            print("  -> WARNING: Could not find ASan/UBSan targets in meson.build to patch")


if __name__ == "__main__":
    main()
