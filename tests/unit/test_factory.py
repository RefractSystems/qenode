"""
SOTA Test Module: test_factory

Context:
This module implements tests for the test_factory subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_factory.
"""

from pathlib import Path
from unittest.mock import MagicMock, patch

from tools.testing.virtmcu_test_suite.factory import compile_c_snippet, compile_dtb, compile_firmware


def test_compile_dtb(tmp_path):
    base_dts = tmp_path / "base.dts"
    base_dts.write_text('/dts-v1/;\n/ {\n    my_node {\n        prop = "REPLACE_ME";\n    };\n};')

    out_dtb = tmp_path / "out.dtb"

    # We patch subprocess.run to avoid needing dtc installed in all test environments
    # but we will check that the temporary file was created correctly.
    def mock_run(args, **_kwargs):
        assert "dtc" in args
        assert "-o" in args
        assert str(out_dtb) in args
        # tmp_dts is the last arg
        tmp_dts = args[-1]
        with Path(tmp_dts).open() as f:
            content = f.read()
            assert "hello" in content
            assert "REPLACE_ME" not in content

        # simulate dtc outputting a file
        out_dtb.write_bytes(b"dummy dtb content")
        return MagicMock(returncode=0)

    with patch("tools.testing.virtmcu_test_suite.factory.subprocess.run", side_effect=mock_run):
        result = compile_dtb(base_dts, {"REPLACE_ME": "hello"}, out_dtb)

    assert result == out_dtb
    assert out_dtb.exists()


def test_compile_firmware(tmp_path):
    src = tmp_path / "main.c"
    src.write_text("int main() { return 0; }")
    out_elf = tmp_path / "main.elf"
    linker = tmp_path / "link.ld"

    def mock_run(args, **_kwargs):
        assert "arm-none-eabi-gcc" in args
        assert "-mcpu=cortex-a15" in args
        assert "-T" in args
        assert str(linker) in args
        assert str(src) in args
        assert "-o" in args
        assert str(out_elf) in args
        out_elf.write_bytes(b"dummy elf")
        return MagicMock(returncode=0)

    with patch("tools.testing.virtmcu_test_suite.factory.subprocess.run", side_effect=mock_run):
        result = compile_firmware([src], out_elf, linker_script=linker)

    assert result == out_elf
    assert out_elf.exists()


def test_compile_c_snippet(tmp_path):
    def mock_run(args, **_kwargs):
        out_elf = args[args.index("-o") + 1]
        Path(out_elf).write_bytes(b"dummy elf")
        return MagicMock(returncode=0)

    with patch("tools.testing.virtmcu_test_suite.factory.subprocess.run", side_effect=mock_run):
        out_elf = compile_c_snippet("int main() { return 0; }", tmp_path)

    assert out_elf.exists()
    assert (tmp_path / "snippet.c").exists()
    assert "int main()" in (tmp_path / "snippet.c").read_text()
