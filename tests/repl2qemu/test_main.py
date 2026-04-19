import os
import sys
from unittest.mock import patch

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
