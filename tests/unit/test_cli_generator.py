"""
SOTA Test Module: test_cli_generator

Context:
This module implements tests for the test_cli_generator subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_cli_generator.
"""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent / "../../"))
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
