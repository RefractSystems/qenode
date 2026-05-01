import logging

import pytest

logger = logging.getLogger(__name__)


@pytest.mark.asyncio
async def test_qmp_basic_communication(qmp_bridge):
    """
    Test that we can send a simple command and get a response.
    """
    res = await qmp_bridge.execute("query-version")
    assert "qemu" in res
    logger.info(f"Connected to QEMU version: {res['qemu']['major']}.{res['qemu']['minor']}")


@pytest.mark.asyncio
async def test_uart_hi_output(qmp_bridge):
    """
    Verify that the 'hello.elf' firmware prints 'HI' to the UART.
    """
    # Note: QEMU might have already printed it before we connected
    # but our bridge starts reading from the socket immediately.
    # If it's already in the buffer, this should return True immediately.
    found = await qmp_bridge.wait_for_line_on_uart("HI", timeout=5.0)
    assert found, f"Did not find 'HI' in UART buffer. Current buffer: {qmp_bridge.uart_buffer!r}"


@pytest.mark.asyncio
async def test_pc_retrieval(qmp_bridge):
    """
    Test that we can retrieve the Program Counter.
    """
    pc = await qmp_bridge.get_pc()
    logger.info(f"Current PC: {hex(pc)}")
    # In arm-generic-fdt, RAM usually starts at 0x40000000
    assert pc >= 0x40000000, f"PC {hex(pc)} is below expected RAM start"


@pytest.mark.asyncio
async def test_emulation_control(qemu_launcher):
    """
    Test pausing, resuming, and resetting emulation.
    """
    # Launch QEMU paused (-S)
    bridge = await qemu_launcher(
        "tests/fixtures/guest_apps/boot_arm/minimal.dtb", "tests/fixtures/guest_apps/boot_arm/hello.elf", extra_args=["-S"]
    )

    # Check that it's actually paused
    status = await bridge.execute("query-status")
    assert status["running"] is False

    # Clear buffer and start
    bridge.clear_uart_buffer()
    await bridge.start_emulation()

    # Verify it's running
    status = await bridge.execute("query-status")
    assert status["running"] is True

    # Wait for output
    assert await bridge.wait_for_line_on_uart("HI", timeout=5.0)

    # Pause it
    await bridge.pause_emulation()
    status = await bridge.execute("query-status")
    assert status["running"] is False

    # Reset it
    bridge.clear_uart_buffer()
    await bridge.execute("system_reset")
    # After system_reset in QMP, it remains in the same running/paused state as before
    # or it might auto-start if not -S? Actually QMP system_reset doesn't change running state.
    await bridge.start_emulation()
    assert await bridge.wait_for_line_on_uart("HI", timeout=5.0)


@pytest.mark.asyncio
async def test_hmp_command(qmp_bridge):
    """
    Test that we can execute Human Monitor Commands (HMP).
    """
    res = await qmp_bridge.execute("human-monitor-command", {"command-line": "info version"})
    # QEMU version string can be '11.0.0' or 'v11.0.0' etc.
    assert "11.0.0" in res


@pytest.mark.asyncio
async def test_uart_write(qmp_bridge):
    """
    Test that we can write data to the UART.
    Since 'hello.elf' doesn't echo, we just verify the socket write doesn't raise an error.
    """
    try:
        await qmp_bridge.write_to_uart("Test string\n")
    except Exception as e:
        pytest.fail(f"Writing to UART raised an exception: {e}")
