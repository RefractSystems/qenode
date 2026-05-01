"""
SOTA Test Module: test_main

Context:
This module implements tests for the test_main subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_main.
"""

import subprocess
import sys
from pathlib import Path
from unittest.mock import patch

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent / "../../"))
from tools.repl2qemu.__main__ import main


def test_main_cli(tmp_path):
    repl_file = tmp_path / "test.repl"
    dtb_file = tmp_path / "test.dtb"
    repl_file.write_text("sram: Memory.MappedMemory @ sysbus 0x20000000\n    size: 0x1000\n")

    test_args = ["repl2qemu", str(repl_file), "--out-dtb", str(dtb_file), "--print-cmd"]
    with patch.object(sys, "argv", test_args), patch("tools.repl2qemu.__main__.compile_dtb", return_value=True):
        main()

    assert not Path(str(dtb_file) + ".dts").exists()  # cleaned up


def test_main_file_not_found(caplog):
    test_args = ["repl2qemu", "does_not_exist.repl", "--out-dtb", "out.dtb"]
    with patch.object(sys, "argv", test_args):
        with pytest.raises(SystemExit) as e:
            main()
        assert e.value.code == 1
    assert "Error: File 'does_not_exist.repl' not found" in caplog.text


def test_main_compile_fails(tmp_path, caplog):
    repl_file = tmp_path / "test.repl"
    dtb_file = tmp_path / "test.dtb"
    repl_file.write_text("sram: Memory.MappedMemory @ sysbus 0x20000000\n    size: 0x1000\n")

    test_args = ["repl2qemu", str(repl_file), "--out-dtb", str(dtb_file)]
    with patch.object(sys, "argv", test_args), patch("tools.repl2qemu.__main__.compile_dtb", return_value=False):
        with pytest.raises(SystemExit) as e:
            main()
        assert e.value.code == 1
    assert "FAILED." in caplog.text


def test_main_out_arch(tmp_path):
    repl_file = tmp_path / "test.repl"
    dtb_file = tmp_path / "test.dtb"
    arch_file = tmp_path / "test.arch"
    repl_file.write_text("sram: Memory.MappedMemory @ sysbus 0x20000000\n    size: 0x1000\n")

    test_args = ["repl2qemu", str(repl_file), "--out-dtb", str(dtb_file), "--out-arch", str(arch_file)]
    with patch.object(sys, "argv", test_args), patch("tools.repl2qemu.__main__.compile_dtb", return_value=True):
        main()

    assert arch_file.read_text() == "arm"  # default arch


def test_main_module():
    result = subprocess.run(
        [sys.executable, "-m", "tools.repl2qemu", "--help"],
        capture_output=True,
        text=True,
        cwd=Path(Path(Path(__file__).resolve().parent) / "../../"),
    )
    assert result.returncode == 0
    assert "Convert Renode .repl to QEMU Device Tree" in result.stdout
