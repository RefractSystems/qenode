import os
import time

import pytest

from tools.testing.QemuLibrary import QemuLibrary


def test_qemu_library_init():
    lib = QemuLibrary()
    assert lib.bridge is not None
    assert lib.loop is not None

def test_qemu_library_launch_and_close():
    lib = QemuLibrary()
    try:
        # Use a minimal DTB for launching
        dtb = "test/phase1/minimal.dtb"
        kernel = "test/phase1/hello.elf"
        if not os.path.exists(dtb):
            pytest.skip("Minimal DTB not found")

        qmp_sock, uart_sock = lib.launch_qemu(dtb, kernel_path=kernel, extra_args=["-S"])

        assert os.path.exists(qmp_sock)
        assert os.path.exists(uart_sock)

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

def test_qemu_library_pc_assertion():
    lib = QemuLibrary()
    try:
        dtb = "test/phase1/minimal.dtb"
        kernel = "test/phase1/hello.elf"
        if not os.path.exists(dtb):
            pytest.skip("Minimal DTB not found")

        qmp_sock, uart_sock = lib.launch_qemu(dtb, kernel_path=kernel, extra_args=["-S"])
        lib.connect_to_qemu(qmp_sock, uart_sock)

        # QEMU sets PC to entry point when loading ELF, even if paused
        lib.pc_should_be_equal(0x40000000)

        lib.start_emulation()
        # Wait a bit
        time.sleep(0.5)
        # Should still be in RAM (or at same address if it's a tight loop)
        actual_pc = lib.loop.run_until_complete(lib.bridge.get_pc())
        assert actual_pc >= 0x40000000, f"Expected PC >= 0x40000000, but was {hex(actual_pc)}"

        with pytest.raises(AssertionError):
            lib.pc_should_be_equal(0x0)
    finally:
        lib.close_all_connections()

def test_qemu_library_uart_wait_fail():
    lib = QemuLibrary()
    try:
        dtb = "test/phase1/minimal.dtb"
        kernel = "test/phase1/hello.elf"
        if not os.path.exists(dtb):
            pytest.skip("Minimal DTB not found")

        qmp_sock, uart_sock = lib.launch_qemu(dtb, kernel_path=kernel, extra_args=["-S"])

        lib.connect_to_qemu(qmp_sock, uart_sock)

        with pytest.raises(AssertionError):
            lib.wait_for_line_on_uart("NEVER_GOING_TO_HAPPEN", timeout=0.1)
    finally:
        lib.close_all_connections()
