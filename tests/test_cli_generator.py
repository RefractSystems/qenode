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
    platform.devices.append(
        ReplDevice(name="cpu0", type_name=cpu_type, address_str="sysbus", properties={})
    )
    return platform


# ── DTB / machine flag ────────────────────────────────────────────────────────


def test_machine_flag_present():
    platform = ReplPlatform()
    args = generate_cli(platform, "/tmp/test.dtb")
    assert "-M" in args


def test_dtb_path_in_machine_flag():
    platform = ReplPlatform()
    args = generate_cli(platform, "/opt/boards/stm32.dtb")
    joined = " ".join(args)
    assert "hw-dtb=/opt/boards/stm32.dtb" in joined


def test_nographic_always_present():
    platform = ReplPlatform()
    args = generate_cli(platform, "/tmp/test.dtb")
    assert "-nographic" in args


# ── CPU type → accelerator mapping ───────────────────────────────────────────


def test_cortex_m_forces_tcg():
    platform = platform_with_cpu("CPU.CortexM")
    args = generate_cli(platform, "/tmp/test.dtb")
    assert "-accel" in args
    assert args[args.index("-accel") + 1] == "tcg"


def test_default_platform_uses_tcg():
    """An empty platform (no CPU device) must still emit -accel tcg."""
    platform = ReplPlatform()
    args = generate_cli(platform, "/tmp/test.dtb")
    assert "-accel" in args
    assert args[args.index("-accel") + 1] == "tcg"


def test_cortex_a_uses_tcg():
    """Cortex-A defaults to TCG (KVM/hvf detection deferred per ADR-009)."""
    platform = platform_with_cpu("CPU.CortexA")
    args = generate_cli(platform, "/tmp/test.dtb")
    assert "-accel" in args
    assert args[args.index("-accel") + 1] == "tcg"


# ── Argument list integrity ───────────────────────────────────────────────────


def test_returns_list_of_strings():
    platform = ReplPlatform()
    args = generate_cli(platform, "/tmp/test.dtb")
    assert isinstance(args, list)
    assert all(isinstance(a, str) for a in args)


def test_no_empty_args():
    platform = ReplPlatform()
    args = generate_cli(platform, "/tmp/test.dtb")
    assert all(a != "" for a in args)
