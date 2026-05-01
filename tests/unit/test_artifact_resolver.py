"""
SOTA Test Module: test_artifact_resolver

Context:
This module implements tests for the test_artifact_resolver subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_artifact_resolver.
"""

from unittest.mock import patch

import pytest

from tools.testing.virtmcu_test_suite.artifact_resolver import get_rust_binary_path, resolve_rust_binary


def test_get_rust_binary_path_cargo_target_dir(monkeypatch, tmp_path):
    cargo_target = tmp_path / "cargo_target"
    cargo_target.mkdir()
    monkeypatch.setenv("CARGO_TARGET_DIR", str(cargo_target))

    # Test when file exists in CARGO_TARGET_DIR
    dummy_bin = cargo_target / "release/dummy_bin"
    dummy_bin.parent.mkdir(parents=True, exist_ok=True)
    dummy_bin.touch()

    resolved = get_rust_binary_path("dummy_bin")
    assert resolved == dummy_bin


@patch("tools.testing.virtmcu_test_suite.artifact_resolver.Path.exists")
def test_get_rust_binary_path_workspace_fallback(mock_exists, monkeypatch):
    monkeypatch.delenv("CARGO_TARGET_DIR", raising=False)

    # Simulate that only the default workspace target path exists
    def side_effect():
        return True

    # Just check that if nothing exists it returns the default workspace path
    mock_exists.return_value = False
    resolved = get_rust_binary_path("dummy_bin")
    assert resolved.parts[-3:] == ("target", "release", "dummy_bin")


def test_resolve_rust_binary_missing():
    with pytest.raises(FileNotFoundError, match=r"Did you run 'cargo build'\?"):
        resolve_rust_binary("some_nonexistent_binary_12345")
