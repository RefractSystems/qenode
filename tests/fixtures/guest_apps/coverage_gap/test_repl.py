import subprocess
import tempfile
from pathlib import Path

import pytest


def test_repl2qemu_standard():
    """
    Test repl2qemu with a standard test repl.
    """
    repl_path = "tests/fixtures/guest_apps/yaml_boot/test_board.repl"
    if not Path(repl_path).exists():
        pytest.skip("test_board.repl not found")

    with tempfile.NamedTemporaryFile(suffix=".dtb", delete=False) as f:
        dtb_path = f.name

    try:
        result = subprocess.run(["repl2qemu", repl_path, "--out-dtb", dtb_path], capture_output=True, text=True)

        assert result.returncode == 0
        assert Path(dtb_path).exists()
        assert Path(dtb_path).stat().st_size > 0
    finally:
        if Path(dtb_path).exists():
            Path(dtb_path).unlink()


def test_repl2qemu_missing_file():
    """
    Test repl2qemu with a missing file.
    """
    result = subprocess.run(
        ["repl2qemu", "/tmp/non_existent.repl", "--out-dtb", "/tmp/test.dtb"], capture_output=True, text=True
    )
    assert result.returncode != 0
    assert "not found" in result.stderr.lower()
