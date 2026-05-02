"""
Centralized constants and registries for the VirtMCU simulation framework.
"""

from __future__ import annotations

from enum import Enum
from pathlib import Path


class VirtmcuBinary(Enum):
    """
    Registry of all core Rust binaries used in the VirtMCU simulation.
    Maps logical names to their actual binary names.
    """

    DETERMINISTIC_COORDINATOR = ("deterministic_coordinator", "tools/deterministic_coordinator")
    RESD_REPLAY = ("resd_replay", "tools/cyber_bridge")
    MUJOCO_BRIDGE = ("mujoco_bridge", "tools/cyber_bridge")
    CYBER_BRIDGE = ("cyber_bridge", "tools/cyber_bridge")
    STRESS_ADAPTER = ("stress_adapter", "tools/stress_adapter")
    SYSTEMC_ADAPTER = ("systemc_adapter", "tools/systemc_adapter")

    # Legacy Aliases
    ZENOH_COORDINATOR = ("deterministic_coordinator", "tools/deterministic_coordinator")

    def __init__(self, binary_name: str, source_rel_path: str) -> None:
        self._binary_name = binary_name
        self._source_rel_path = source_rel_path

    @property
    def binary_name(self) -> str:
        """Returns the actual filename of the binary (excluding .exe on non-Windows)."""
        return self._binary_name

    @classmethod
    def from_string(cls, name: str) -> VirtmcuBinary:
        """Resolves a string name to a VirtmcuBinary enum, handling legacy aliases."""
        for member in cls:
            if member.binary_name == name or member.name == name:
                return member
        raise ValueError(f"Unknown VirtmcuBinary: {name}")

    def source_path(self, workspace_root: Path) -> Path:
        """Returns the absolute path to the tool's source directory."""
        return workspace_root / self._source_rel_path
