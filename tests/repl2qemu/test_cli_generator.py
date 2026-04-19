import os
import sys

sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "../../")))
from tools.repl2qemu.cli_generator import generate_cli
from tools.repl2qemu.parser import ReplDevice, ReplPlatform


def test_generate_cli_arm():
    plat = ReplPlatform(devices=[ReplDevice(name="cpu", type_name="CPU.CortexM")])
    args, arch = generate_cli(plat, "test.dtb")
    assert arch == "arm"
    assert "-M" in args
    assert "arm-generic-fdt" in args[args.index("-M") + 1]


def test_generate_cli_riscv():
    plat = ReplPlatform(devices=[ReplDevice(name="cpu", type_name="CPU.RISCV64")])
    args, arch = generate_cli(plat, "test.dtb")
    assert arch == "riscv"
    assert "virt" in args[args.index("-M") + 1]
