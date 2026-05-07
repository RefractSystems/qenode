"""
Finds the expected path for a built Rust binary across standard workspace locations.
It returns the path even if the file doesn't exist yet, prioritizing locations
where it actually exists if multiple are possible.
"""

from __future__ import annotations

import contextlib
import logging
import os
import shutil
import subprocess
import warnings
from pathlib import Path

from tools.testing.env import WORKSPACE_DIR
from tools.testing.virtmcu_test_suite.constants import VirtmcuBinary

logger = logging.getLogger(__name__)


def get_host_triple() -> str | None:
    """Gets the Rust host triple."""
    rustc_path = shutil.which("rustc")
    if not rustc_path:
        return None
    try:
        output = subprocess.check_output([rustc_path, "-vV"], stderr=subprocess.DEVNULL).decode()
        for line in output.splitlines():
            if line.startswith("host: "):
                return line.split("host: ")[1].strip()
    except (subprocess.SubprocessError, FileNotFoundError, IndexError):
        pass
    return None


def get_rust_binary_path(name: VirtmcuBinary | str) -> Path:
    """
    Finds the expected path for a built Rust binary across standard workspace locations.
    Prioritizes:
    1. CARGO_TARGET_DIR/release/<name> (if env var set)
    2. WORKSPACE_DIR/target/release/<name>
    3. tools/<name>/target/release/<name>
    4. System PATH (via shutil.which)
    5. Fallback candidate paths
    """
    # 0. Canonicalize the name
    if isinstance(name, VirtmcuBinary):
        bin_name = name.binary_name
    else:
        # Check if the string matches a known binary for deprecation warning
        try:
            matched = VirtmcuBinary.from_string(name)
            warnings.warn(
                f"Usage of hardcoded binary string '{name}' is deprecated. Use VirtmcuBinary.{matched.name} instead.",
                DeprecationWarning,
                stacklevel=2,
            )
            bin_name = matched.binary_name
        except ValueError:
            bin_name = name

    # Determine build suffix based on environment
    build_suffix = ""
    if os.environ.get("VIRTMCU_USE_ASAN") == "1":
        build_suffix = "-asan"
    elif os.environ.get("VIRTMCU_USE_TSAN") == "1":
        build_suffix = "-tsan"

    # 1. Check CARGO_TARGET_DIR if set
    if "CARGO_TARGET_DIR" in os.environ:
        target_dir = Path(os.environ["CARGO_TARGET_DIR"])
        triple = get_host_triple()
        p = target_dir / f"release/{bin_name}"
        if p.exists():
            return p
        if triple:
            p = target_dir / triple / f"release/{bin_name}"
            if p.exists():
                return p

    # 2. Candidate paths within the workspace
    triple = get_host_triple()
    paths = []
    if triple:
        paths.append(WORKSPACE_DIR / f"target{build_suffix}/{triple}/release" / bin_name)

    paths.extend(
        [
            WORKSPACE_DIR / f"target{build_suffix}/release" / bin_name,
            WORKSPACE_DIR / "target/release" / bin_name,
        ]
    )

    # If we have a known binary, check its specific target directory
    registry_match: VirtmcuBinary | None = None
    if isinstance(name, VirtmcuBinary):
        registry_match = name
    else:
        with contextlib.suppress(ValueError):
            registry_match = VirtmcuBinary.from_string(name)

    if registry_match:
        source_path = registry_match.source_path(WORKSPACE_DIR)
        if triple:
            paths.append(source_path / f"target{build_suffix}/{triple}/release/{bin_name}")
        paths.append(source_path / f"target{build_suffix}/release/{bin_name}")
        if build_suffix:
            paths.append(source_path / f"target/release/{bin_name}")

    # Legacy fallback candidates (for unknown binaries or old layouts)
    if triple:
        paths.extend(
            [
                WORKSPACE_DIR / f"tools/{bin_name}/target{build_suffix}/{triple}/release/{bin_name}",
                WORKSPACE_DIR / f"tools/cyber_bridge/target{build_suffix}/{triple}/release/{bin_name}",
                WORKSPACE_DIR / f"tools/deterministic_coordinator/target{build_suffix}/{triple}/release/{bin_name}",
            ]
        )

    paths.extend(
        [
            WORKSPACE_DIR / f"tools/{bin_name}/target{build_suffix}/release/{bin_name}",
            WORKSPACE_DIR / f"tools/cyber_bridge/target{build_suffix}/release/{bin_name}",
            WORKSPACE_DIR / f"tools/deterministic_coordinator/target{build_suffix}/release/{bin_name}",
        ]
    )
    if build_suffix:
        if triple:
            paths.extend(
                [
                    WORKSPACE_DIR / f"tools/{bin_name}/target/{triple}/release/{bin_name}",
                    WORKSPACE_DIR / f"tools/cyber_bridge/target/{triple}/release/{bin_name}",
                    WORKSPACE_DIR / f"tools/deterministic_coordinator/target/{triple}/release/{bin_name}",
                ]
            )
        paths.extend(
            [
                WORKSPACE_DIR / f"tools/{bin_name}/target/release/{bin_name}",
                WORKSPACE_DIR / f"tools/cyber_bridge/target/release/{bin_name}",
                WORKSPACE_DIR / f"tools/deterministic_coordinator/target/release/{bin_name}",
            ]
        )

    for p in paths:
        if p.exists():
            return p

    # 3. Check system PATH
    path_bin = shutil.which(bin_name)
    if path_bin:
        return Path(path_bin)

    # 4. Fallback to standard target dir if it doesn't exist anywhere
    if "CARGO_TARGET_DIR" in os.environ:
        target_dir = Path(os.environ["CARGO_TARGET_DIR"])
        if triple:
            return target_dir / triple / f"release/{bin_name}"
        return target_dir / f"release/{bin_name}"
    return WORKSPACE_DIR / "target/release" / bin_name


def resolve_rust_binary(name: VirtmcuBinary | str) -> Path:
    """
    Finds a built Rust binary across standard workspace locations.
    Raises FileNotFoundError if it doesn't exist.
    """
    p = get_rust_binary_path(name)
    if not p.exists():
        # Ensure we use the canonical name in the error message
        msg_name = name.binary_name if isinstance(name, VirtmcuBinary) else name
        raise FileNotFoundError(f"Binary {msg_name} not found. Searched path: {p}. Did you run 'cargo build'?")
    return p


def resolve_qemu_binary(arch: str = "arm") -> Path:
    """
    Finds the appropriate QEMU binary for the given architecture.
    Prioritizes:
    1. QEMU_BIN environment variable
    2. Local build directory (third_party/qemu/build-virtmcu...)
    3. CI build directory (/build/qemu/build-virtmcu...)
    4. System PATH
    """
    if "QEMU_BIN" in os.environ:
        return Path(os.environ["QEMU_BIN"])

    # Map logical arch to QEMU binary suffix
    qemu_arch = arch
    if arch in ["riscv", "riscv64"]:
        qemu_arch = "riscv64"
    elif arch == "riscv32":
        qemu_arch = "riscv32"

    bin_name = f"qemu-system-{qemu_arch}"

    # Determine build directory name
    build_suffix = ""
    if os.environ.get("VIRTMCU_USE_ASAN") == "1":
        build_suffix = "-asan"
    elif os.environ.get("VIRTMCU_USE_TSAN") == "1":
        build_suffix = "-tsan"

    build_dir_name = f"build-virtmcu{build_suffix}"

    # Candidate paths
    candidates = [
        WORKSPACE_DIR / "third_party/qemu" / build_dir_name / "install/bin" / bin_name,
        WORKSPACE_DIR / "third_party/qemu" / build_dir_name / bin_name,
        Path("/build/qemu") / build_dir_name / "install/bin" / bin_name,
        # Fallback to non-sanitized builds if sanitized not found
        WORKSPACE_DIR / "third_party/qemu/build-virtmcu/install/bin" / bin_name,
        Path("/build/qemu/build-virtmcu/install/bin") / bin_name,
    ]

    for p in candidates:
        if p.exists():
            return p

    # System PATH
    path_bin = shutil.which(bin_name)
    if path_bin:
        return Path(path_bin)

    # Final fallback (doesn't have to exist)
    return WORKSPACE_DIR / "third_party/qemu" / build_dir_name / "install/bin" / bin_name
