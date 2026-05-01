"""
SOTA Test Module: test_qemu_library_pytest

Context:
This module implements tests for the test_qemu_library_pytest subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_qemu_library_pytest.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from tools.testing.QemuLibrary import QemuLibrary
from tools.testing.utils import mock_execution_delay


def test_qemu_library_init() -> None:
    lib = QemuLibrary()
    assert lib.bridge is not None
    assert lib.loop is not None


def test_qemu_library_launch_and_close() -> None:
    lib = QemuLibrary()
    try:
        # Use a minimal DTB for launching
        dtb = "tests/fixtures/guest_apps/boot_arm/minimal.dtb"
        kernel = "tests/fixtures/guest_apps/boot_arm/hello.elf"
        if not Path(dtb).exists():
            pytest.skip("Minimal DTB not found")

        qmp_sock, uart_sock = lib.launch_qemu(dtb, kernel_path=kernel, extra_args=["-S"])

        assert Path(qmp_sock).exists()
        assert Path(uart_sock).exists()

        lib.connect_to_qemu(qmp_sock, uart_sock)
        lib.start_emulation()
        lib.pause_emulation()
        lib.reset_emulation()

        # Test HMP
        res = lib.execute_monitor_command("info version")
        assert "11.0.0" in res or "v11.0.0" in res
    finally:
        lib.close_all_connections()
    assert lib.proc is None
    assert lib.tmpdir is None


def test_qemu_library_pc_assertion() -> None:
    lib = QemuLibrary()
    try:
        dtb = "tests/fixtures/guest_apps/boot_arm/minimal.dtb"
        kernel = "tests/fixtures/guest_apps/boot_arm/hello.elf"
        if not Path(dtb).exists():
            pytest.skip("Minimal DTB not found")

        qmp_sock, uart_sock = lib.launch_qemu(dtb, kernel_path=kernel, extra_args=["-S"])
        lib.connect_to_qemu(qmp_sock, uart_sock)

        # QEMU sets PC to entry point when loading ELF, even if paused
        lib.pc_should_be_equal(0x40000000)

        lib.start_emulation()
        # Wait a bit for execution to proceed in wall-clock mode
        mock_execution_delay(0.5)  # SLEEP_EXCEPTION: waiting for wall-clock execution (legacy interface)
        # Should still be in RAM (or at same address if it's a tight loop)
        actual_pc = lib.loop.run_until_complete(lib.bridge.get_pc())
        assert actual_pc >= 0x40000000, f"Expected PC >= 0x40000000, but was {hex(actual_pc)}"

        with pytest.raises(AssertionError, match=r".*"):
            lib.pc_should_be_equal(0x0)
    finally:
        lib.close_all_connections()


def test_qemu_library_uart_wait_fail() -> None:
    lib = QemuLibrary()
    try:
        dtb = "tests/fixtures/guest_apps/boot_arm/minimal.dtb"
        kernel = "tests/fixtures/guest_apps/boot_arm/hello.elf"
        if not Path(dtb).exists():
            pytest.skip("Minimal DTB not found")

        qmp_sock, uart_sock = lib.launch_qemu(dtb, kernel_path=kernel, extra_args=["-S"])

        lib.connect_to_qemu(qmp_sock, uart_sock)

        assert lib is not None
        with pytest.raises(AssertionError, match=r".*"):
            lib.wait_for_line_on_uart("NEVER_GOING_TO_HAPPEN", timeout=0.1)
    finally:
        lib.close_all_connections()
