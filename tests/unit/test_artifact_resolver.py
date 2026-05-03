"""
SOTA Test Module: test_artifact_resolver

Context:
This module implements tests for the test_artifact_resolver subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_artifact_resolver.
"""

from __future__ import annotations

from typing import TYPE_CHECKING
from unittest.mock import MagicMock, patch

import pytest

from tools.testing.virtmcu_test_suite.artifact_resolver import get_rust_binary_path, resolve_rust_binary
from tools.testing.virtmcu_test_suite.constants import VirtmcuBinary

if TYPE_CHECKING:
    from pathlib import Path

    pass


def test_get_rust_binary_path_cargo_target_dir(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    cargo_target = tmp_path / "cargo_target"
    cargo_target.mkdir()
    monkeypatch.setenv("CARGO_TARGET_DIR", str(cargo_target))

    # Test when file exists in CARGO_TARGET_DIR
    dummy_bin = cargo_target / "release/dummy_bin"
    dummy_bin.parent.mkdir(parents=True, exist_ok=True)
    dummy_bin.touch()

    resolved = get_rust_binary_path("dummy_bin")  # LINT_EXCEPTION: hardcoded_binary
    assert resolved == dummy_bin


@patch("tools.testing.virtmcu_test_suite.artifact_resolver.Path.exists")
def test_get_rust_binary_path_workspace_fallback(mock_exists: MagicMock, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("CARGO_TARGET_DIR", raising=False)

    # Simulate that only the default workspace target path exists
    def side_effect() -> bool:
        return True

    # Just check that if nothing exists it returns the default workspace path
    mock_exists.return_value = False
    resolved = get_rust_binary_path("dummy_bin")  # LINT_EXCEPTION: hardcoded_binary
    assert resolved.parts[-3:] == ("target", "release", "dummy_bin")


def test_resolve_rust_binary_missing() -> None:
    with pytest.raises(FileNotFoundError, match=r"Did you run 'cargo build'\?"):
        resolve_rust_binary("some_nonexistent_binary_12345")  # LINT_EXCEPTION: hardcoded_binary


def test_get_rust_binary_path_enum() -> None:
    # Testing with a known binary from the Enum
    resolved = get_rust_binary_path(VirtmcuBinary.DETERMINISTIC_COORDINATOR)
    assert resolved.name == "deterministic_coordinator"
    assert "deterministic_coordinator" in str(resolved)
