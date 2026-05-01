#!/usr/bin/env python3
"""
Check QOM TypeInfo / DTS / Meson / pytest -global name alignment.

A QOM type name registered by a Rust plugin MUST be identical across:
  1. hw/rust/<crate>/src/lib.rs   — TypeInfo `name: c"<X>".as_ptr()`
  2. hw/meson.build — `'obj': '<X>'`
  3. tests/fixtures/.../<X>.dts   — `compatible = "<X>";`
  4. tests/integration/**/test_*.py — `-global <X>.<prop>=<val>`

A mismatch causes one of two failures:
  * QEMU's fdt_generic_util.c can't find the type, device is silently skipped
    (then -global properties never apply).
  * QEMU's type_get_or_load_by_name() finds the modinfo entry but can't load
    the .so, then NULL-derefs in error_vprepend (rc=-11 SIGSEGV).

Exit non-zero on any mismatch. Prints actionable remediation.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

from tools.testing.env import WORKSPACE_DIR

ROOT = WORKSPACE_DIR
RUST_PLUGIN_ROOTS = [
    ROOT / "hw/rust/comms",
    ROOT / "hw/rust/mcu",
    ROOT / "hw/rust/observability",
    ROOT / "hw/rust/backbone",
]
MESON_BUILD = ROOT / "hw/meson.build"
DTS_ROOT = ROOT / "tests/fixtures/guest_apps"
TEST_ROOT = ROOT / "tests/integration"

TYPE_INFO_RE = re.compile(
    r'name:\s*c"([^"]+)"\.as_ptr\(\)',
    re.MULTILINE,
)
MESON_OBJ_RE = re.compile(
    r"\{\s*'pkg':\s*'([^']+)'\s*,\s*'lib':\s*'([^']+)'\s*,\s*'obj':\s*'([^']+)'",
    re.MULTILINE,
)
DTS_COMPAT_RE = re.compile(r'compatible\s*=\s*"([^"]+)"\s*;')
GLOBAL_RE = re.compile(r'"-global"\s*,\s*f?"([a-zA-Z0-9_,.-]+)\.[a-zA-Z0-9_-]+=')


def find_rust_types() -> dict[str, list[Path]]:
    """Return {qom_type_name: [lib.rs files]}."""
    out: dict[str, list[Path]] = {}
    for root in RUST_PLUGIN_ROOTS:
        if not root.exists():
            continue
        for lib_rs in root.rglob("src/lib.rs"):
            content = lib_rs.read_text(encoding="utf-8")
            for m in TYPE_INFO_RE.finditer(content):
                if "TYPE_INFO" not in content[max(0, m.start() - 200) : m.start()]:
                    continue
                out.setdefault(m.group(1), []).append(lib_rs)
    return out


def find_meson_objs() -> set[str]:
    """Return set of `obj` strings from meson.build."""
    if not MESON_BUILD.exists():
        return set()
    content = MESON_BUILD.read_text(encoding="utf-8")
    return {m.group(3) for m in MESON_OBJ_RE.finditer(content)}


def find_dts_compatibles() -> dict[str, list[Path]]:
    """Return {compatible: [dts files]} from peripheral nodes."""
    out: dict[str, list[Path]] = {}
    if not DTS_ROOT.exists():
        return out
    for dts in DTS_ROOT.rglob("*.dts"):
        for m in DTS_COMPAT_RE.finditer(dts.read_text(encoding="utf-8")):
            out.setdefault(m.group(1), []).append(dts)
    return out


def find_test_globals() -> dict[str, list[Path]]:
    """Return {-global prefix: [test files]}."""
    out: dict[str, list[Path]] = {}
    if not TEST_ROOT.exists():
        return out
    for test in TEST_ROOT.rglob("test_*.py"):
        for m in GLOBAL_RE.finditer(test.read_text(encoding="utf-8")):
            out.setdefault(m.group(1), []).append(test)
    return out


# Names known to be QEMU built-in types (CPUs, machines, standard peripherals,
# riscv core IP, etc.), not Rust plugins under hw/rust/.
NON_PLUGIN_COMPATIBLES = {
    "arm,generic-fdt",
    "qemu:system-memory",
    "qemu-memory-region",
    "cortex-a15-arm-cpu",
    "riscv,rv32",
    "riscv,rv64",
    "riscv-virtio",
    "riscv,cpu-intc",
    "ns16550a",
    "pl011",
    "memory",
    "simple-bus",
}


def main() -> int:
    rust_types = find_rust_types()
    meson_objs = find_meson_objs()
    dts_compats = find_dts_compatibles()
    test_globals = find_test_globals()

    errors: list[str] = []

    for qom_type, files in rust_types.items():
        if qom_type not in meson_objs:
            errors.append(
                f"Rust TypeInfo `{qom_type}` (registered in {files[0].relative_to(ROOT)}) "
                f"has no matching `'obj': '{qom_type}'` in {MESON_BUILD.relative_to(ROOT)}.\n"
                f"  Fix: add an entry to hw/virtmcu/meson.build OR update lib.rs name."
            )

    for compat, files in dts_compats.items():
        if compat in NON_PLUGIN_COMPATIBLES:
            continue
        if compat not in rust_types and compat not in meson_objs:
            files_str = ", ".join(str(f.relative_to(ROOT)) for f in files[:3])
            errors.append(
                f'DTS `compatible = "{compat}"` (in {files_str}) has no matching '
                f"Rust TypeInfo or Meson `obj`.\n"
                f"  Fix: rename DTS to a registered type, or register `{compat}` in lib.rs+meson.build."
            )

    for prefix, files in test_globals.items():
        if prefix not in rust_types and prefix not in meson_objs:
            files_str = ", ".join(str(f.relative_to(ROOT)) for f in files[:3])
            errors.append(
                f"pytest `-global {prefix}.*` (in {files_str}) targets a type "
                f"with no matching Rust TypeInfo or Meson `obj`.\n"
                f"  Fix: align -global prefix with the actual QOM type name."
            )

    if errors:
        sys.stderr.write("\nQOM ALIGNMENT ERRORS:\n")
        for e in errors:
            sys.stderr.write(f"\n  {e}\n")
        sys.stderr.write(
            "\nWhy this matters: a mismatch causes QEMU's module loader to "
            "fail in a path that NULL-derefs error_vprepend (SIGSEGV / rc=-11).\n"
        )
        return 1

    return 0


if __name__ == "__main__":
    sys.exit(main())
