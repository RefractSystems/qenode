"""
tests/test_cli_generator.py — Unit tests for tools/repl2qemu/cli_generator.py

Tests CLI argument generation in isolation (no QEMU binary needed).
Verifies that the correct flags are produced for different platform types.
"""

import os
import sys

# Import via the package so relative imports inside cli_generator resolve.
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "..")))
from tools.repl2qemu.cli_generator import generate_cli
from tools.repl2qemu.parser import ReplDevice, ReplPlatform

# ── Helpers ───────────────────────────────────────────────────────────────────


def platform_with_cpu(cpu_type: str) -> ReplPlatform:
    platform = ReplPlatform()
    platform.devices.append(ReplDevice(name="cpu0", type_name=cpu_type, address_str="sysbus", properties={}))
    return platform


# ── DTB / machine flag ────────────────────────────────────────────────────────


def test_machine_flag_present():
    platform = ReplPlatform()
    args, arch = generate_cli(platform, "/tmp/test.dtb")
    assert "-M" in args


def test_dtb_path_in_machine_flag():
    platform = ReplPlatform()
    args, arch = generate_cli(platform, "/opt/boards/stm32.dtb")
    joined = " ".join(args)
    assert "hw-dtb=/opt/boards/stm32.dtb" in joined


def test_nographic_always_present():
    platform = ReplPlatform()
    args, arch = generate_cli(platform, "/tmp/test.dtb")
    assert "-nographic" in args


# ── CPU type → accelerator mapping ───────────────────────────────────────────


def test_cortex_m_forces_tcg():
    platform = platform_with_cpu("CPU.CortexM")
    args, arch = generate_cli(platform, "/tmp/test.dtb")
    assert "-accel" in args
    assert args[args.index("-accel") + 1] == "tcg"


def test_default_platform_uses_tcg():
    """An empty platform (no CPU device) must still emit -accel tcg."""
    platform = ReplPlatform()
    args, arch = generate_cli(platform, "/tmp/test.dtb")
    assert "-accel" in args
    assert args[args.index("-accel") + 1] == "tcg"


def test_cortex_a_uses_tcg():
    """Cortex-A defaults to TCG (KVM/hvf detection deferred per ADR-009)."""
    platform = platform_with_cpu("CPU.CortexA")
    args, arch = generate_cli(platform, "/tmp/test.dtb")
    assert "-accel" in args
    assert args[args.index("-accel") + 1] == "tcg"


# ── Architecture detection ────────────────────────────────────────────────────


def test_arm_platform_returns_arm_arch():
    platform = platform_with_cpu("CPU.CortexM")
    args, arch = generate_cli(platform, "/tmp/test.dtb")
    assert arch == "arm"


def test_riscv_platform_returns_riscv_arch():
    platform = platform_with_cpu("CPU.RISCV64")
    args, arch = generate_cli(platform, "/tmp/test.dtb")
    assert arch == "riscv"


def test_riscv_uses_virt_machine():
    platform = platform_with_cpu("CPU.RISCV64")
    args, arch = generate_cli(platform, "/tmp/test.dtb")
    m_idx = args.index("-M")
    assert args[m_idx + 1] == "virt"


def test_riscv_passes_dtb_separately():
    """RISC-V virt uses -dtb, not hw-dtb= in machine name."""
    platform = platform_with_cpu("CPU.RISCV64")
    args, arch = generate_cli(platform, "/tmp/rv.dtb")
    assert "-dtb" in args
    assert args[args.index("-dtb") + 1] == "/tmp/rv.dtb"


# ── Argument list integrity ───────────────────────────────────────────────────


def test_returns_tuple_of_list_and_arch():
    platform = ReplPlatform()
    result = generate_cli(platform, "/tmp/test.dtb")
    assert isinstance(result, tuple)
    assert len(result) == 2
    args, arch = result
    assert isinstance(args, list)
    assert isinstance(arch, str)


def test_no_empty_args():
    platform = ReplPlatform()
    args, arch = generate_cli(platform, "/tmp/test.dtb")
    assert all(a != "" for a in args)
