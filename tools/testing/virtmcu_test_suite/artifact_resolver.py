"""
Finds the expected path for a built Rust binary across standard workspace locations.
It returns the path even if the file doesn't exist yet, prioritizing locations
where it actually exists if multiple are possible.
"""

from __future__ import annotations

import logging
import os
import shutil
import warnings
from pathlib import Path

from tools.testing.env import WORKSPACE_DIR
from tools.testing.virtmcu_test_suite.constants import VirtmcuBinary

logger = logging.getLogger(__name__)


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

    # 1. Check CARGO_TARGET_DIR if set
    if "CARGO_TARGET_DIR" in os.environ:
        p = Path(os.environ["CARGO_TARGET_DIR"]) / f"release/{bin_name}"
        if p.exists():
            return p

    # 2. Candidate paths within the workspace
    paths = [
        WORKSPACE_DIR / "target/release" / bin_name,
    ]

    # If we have a known binary, check its specific target directory
    registry_match: VirtmcuBinary | None = None
    if isinstance(name, VirtmcuBinary):
        registry_match = name
    else:
        try:
            registry_match = VirtmcuBinary.from_string(name)
        except ValueError:
            pass

    if registry_match:
        paths.append(registry_match.source_path(WORKSPACE_DIR) / f"target/release/{bin_name}")

    # Legacy fallback candidates (for unknown binaries or old layouts)
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
        return Path(os.environ["CARGO_TARGET_DIR"]) / f"release/{bin_name}"
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
