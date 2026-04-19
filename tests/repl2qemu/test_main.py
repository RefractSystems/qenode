import os
import subprocess
import sys
from unittest.mock import patch

import pytest

# noqa: E402
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "../../")))
from tools.repl2qemu.__main__ import main


def test_main_cli(tmp_path):
    repl_file = tmp_path / "test.repl"
    dtb_file = tmp_path / "test.dtb"
    repl_file.write_text("sram: Memory.MappedMemory @ sysbus 0x20000000\n    size: 0x1000\n")

    test_args = ["repl2qemu", str(repl_file), "--out-dtb", str(dtb_file), "--print-cmd"]
    with patch.object(sys, "argv", test_args):
        # We need to mock compile_dtb too because it calls 'dtc' which might not be in test env
        with patch("tools.repl2qemu.__main__.compile_dtb", return_value=True):
            main()

    assert not os.path.exists(str(dtb_file) + ".dts")  # cleaned up


def test_main_file_not_found(capsys):
    test_args = ["repl2qemu", "does_not_exist.repl", "--out-dtb", "out.dtb"]
    with patch.object(sys, "argv", test_args):
        with pytest.raises(SystemExit) as e:
            main()
        assert e.value.code == 1
    captured = capsys.readouterr()
    assert "Error: File 'does_not_exist.repl' not found" in captured.err

def test_main_compile_fails(tmp_path, capsys):
    repl_file = tmp_path / "test.repl"
    dtb_file = tmp_path / "test.dtb"
    repl_file.write_text("sram: Memory.MappedMemory @ sysbus 0x20000000\n")

    test_args = ["repl2qemu", str(repl_file), "--out-dtb", str(dtb_file)]
    with patch.object(sys, "argv", test_args):
        with patch("tools.repl2qemu.__main__.compile_dtb", return_value=False):
            with pytest.raises(SystemExit) as e:
                main()
            assert e.value.code == 1
    captured = capsys.readouterr()
    assert "FAILED." in captured.out

def test_main_out_arch(tmp_path):
    repl_file = tmp_path / "test.repl"
    dtb_file = tmp_path / "test.dtb"
    arch_file = tmp_path / "test.arch"
    repl_file.write_text("sram: Memory.MappedMemory @ sysbus 0x20000000\n")

    test_args = ["repl2qemu", str(repl_file), "--out-dtb", str(dtb_file), "--out-arch", str(arch_file)]
    with patch.object(sys, "argv", test_args):
        with patch("tools.repl2qemu.__main__.compile_dtb", return_value=True):
            main()

    assert arch_file.read_text() == "arm"  # default arch

def test_main_module():
    result = subprocess.run([sys.executable, "-m", "tools.repl2qemu", "--help"], capture_output=True, text=True, cwd=os.path.abspath(os.path.join(os.path.dirname(__file__), "../../")))
    assert result.returncode == 0
    assert "Convert Renode .repl to QEMU Device Tree" in result.stdout
