import os
import subprocess
import tempfile

import pytest


def test_repl2qemu_standard():
    """
    Test repl2qemu with a standard test repl.
    """
    repl_path = "test/phase3/test_board.repl"
    if not os.path.exists(repl_path):
        pytest.skip("test_board.repl not found")

    with tempfile.NamedTemporaryFile(suffix=".dtb", delete=False) as f:
        dtb_path = f.name

    try:
        result = subprocess.run(
            ["repl2qemu", repl_path, "--out-dtb", dtb_path],
            capture_output=True,
            text=True
        )

        assert result.returncode == 0
        assert os.path.exists(dtb_path)
        assert os.path.getsize(dtb_path) > 0
    finally:
        if os.path.exists(dtb_path):
            os.remove(dtb_path)

def test_repl2qemu_missing_file():
    """
    Test repl2qemu with a missing file.
    """
    result = subprocess.run(
        ["repl2qemu", "/tmp/non_existent.repl", "--out-dtb", "/tmp/test.dtb"],
        capture_output=True,
        text=True
    )
    assert result.returncode != 0
    assert "not found" in result.stderr.lower()
