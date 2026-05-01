#!/usr/bin/env python3
"""
Verify Cargo package -> Meson `lib` field alignment for every Rust peripheral.

Cargo produces `lib<name>.a` where `<name>` is the package name with hyphens
replaced by underscores (or `[lib].name` if explicitly set). The Meson build
references each lib via a `'lib': 'lib<name>.a'` entry.

A mismatch (e.g. package `flexray` but meson `'lib': 'libflexray.a'`)
causes Meson to silently link a stale or empty static library, producing a
.so file that contains old code or fails to register types — both manifest as
opaque runtime SIGSEGVs.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

from tools.testing.env import WORKSPACE_DIR

ROOT = WORKSPACE_DIR
RUST_ROOTS = [
    ROOT / "hw/rust/comms",
    ROOT / "hw/rust/mcu",
    ROOT / "hw/rust/observability",
    ROOT / "hw/rust/backbone",
]
MESON_BUILD = ROOT / "hw/meson.build"

PKG_NAME_RE = re.compile(r'^name\s*=\s*"([^"]+)"\s*$', re.MULTILINE)
LIB_NAME_RE = re.compile(
    r'^\[lib\]\s*$.*?^name\s*=\s*"([^"]+)"',
    re.MULTILINE | re.DOTALL,
)
MESON_ENTRY_RE = re.compile(
    r"\{\s*'pkg':\s*'([^']+)'\s*,\s*'lib':\s*'([^']+)'\s*,\s*'obj':\s*'([^']+)'",
    re.MULTILINE,
)


def cargo_lib_filename(cargo_toml: Path) -> tuple[str, str]:
    """Return (pkg_name, expected_lib_filename) for a Cargo.toml."""
    content = cargo_toml.read_text(encoding="utf-8")
    pkg_match = PKG_NAME_RE.search(content)
    if not pkg_match:
        return ("", "")
    pkg_name = pkg_match.group(1)

    lib_match = LIB_NAME_RE.search(content)
    lib_name = lib_match.group(1) if lib_match else pkg_name.replace("-", "_")
    return (pkg_name, f"lib{lib_name}.a")


def find_meson_entries() -> dict[str, tuple[str, str]]:
    """Return {pkg_name: (lib_filename, obj)} from meson.build."""
    out: dict[str, tuple[str, str]] = {}
    if not MESON_BUILD.exists():
        return out
    content = MESON_BUILD.read_text(encoding="utf-8")
    for m in MESON_ENTRY_RE.finditer(content):
        out[m.group(1)] = (m.group(2), m.group(3))
    return out


def main() -> int:
    meson_entries = find_meson_entries()
    errors: list[str] = []

    for root in RUST_ROOTS:
        if not root.exists():
            continue
        for cargo_toml in root.rglob("Cargo.toml"):
            pkg_name, expected_lib = cargo_lib_filename(cargo_toml)
            if not pkg_name:
                continue

            if pkg_name not in meson_entries:
                continue
            actual_lib, _obj = meson_entries[pkg_name]

            if actual_lib != expected_lib:
                errors.append(
                    f"Cargo package `{pkg_name}` "
                    f"(in {cargo_toml.relative_to(ROOT)}) produces `{expected_lib}` "
                    f"but {MESON_BUILD.relative_to(ROOT)} expects `{actual_lib}`.\n"
                    f"  Fix: change Meson `'lib': '{actual_lib}'` to `'lib': '{expected_lib}'` "
                    f'OR add `[lib]\\nname = "<custom>"` in {cargo_toml.relative_to(ROOT)}.'
                )

    if errors:
        sys.stderr.write("\nCARGO / MESON LIBRARY-NAME MISMATCHES:\n")
        for e in errors:
            sys.stderr.write(f"\n  {e}\n")
        sys.stderr.write(
            "\nWhy this matters: a stale .a is silently linked, producing a "
            "working-looking .so that contains old type registrations.\n"
        )
        return 1

    return 0


if __name__ == "__main__":
    sys.exit(main())
