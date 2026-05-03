"""
Test that we can send a simple command and get a response.
"""

from __future__ import annotations

import logging
from typing import TYPE_CHECKING, Any, cast

import pytest

if TYPE_CHECKING:
    from tools.testing.qmp_bridge import QmpBridge


logger = logging.getLogger(__name__)


@pytest.mark.asyncio
async def test_qmp_basic_communication(qmp_bridge: QmpBridge) -> None:

    res = await qmp_bridge.execute("query-version")
    assert "qemu" in res  # type: ignore[operator]
    logger.info(f"Connected to QEMU version: {res['qemu']['major']}.{res['qemu']['minor']}")  # type: ignore[index]


@pytest.mark.asyncio
async def test_uart_hi_output(qmp_bridge: QmpBridge) -> None:
    """
    Verify that the 'hello.elf' firmware prints 'HI' to the UART.
    """
    # Note: QEMU might have already printed it before we connected
    # but our bridge starts reading from the socket immediately.
    # If it's already in the buffer, this should return True immediately.
    assert qmp_bridge is not None
    found = await qmp_bridge.wait_for_line_on_uart("HI", timeout=5.0)
    assert qmp_bridge is not None
    assert found, f"Did not find 'HI' in UART buffer. Current buffer: {qmp_bridge.uart_buffer!r}"


@pytest.mark.asyncio
async def test_pc_retrieval(qmp_bridge: QmpBridge) -> None:
    """
    Test that we can retrieve the Program Counter.
    """
    pc = await qmp_bridge.get_pc()
    logger.info(f"Current PC: {hex(pc)}")
    # In arm-generic-fdt, RAM usually starts at 0x40000000
    assert pc >= 0x40000000, f"PC {hex(pc)} is below expected RAM start"


@pytest.mark.asyncio
async def test_emulation_control(inspection_bridge: object) -> None:
    """
    Test pausing, resuming, and resetting emulation.
    """
    # inspection_bridge spawns frozen (-S)
    bridge = await cast(Any, inspection_bridge)(
        "tests/fixtures/guest_apps/boot_arm/minimal.dtb",
        "tests/fixtures/guest_apps/boot_arm/hello.elf",
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
    assert bridge is not None
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
    assert bridge is not None
    assert await bridge.wait_for_line_on_uart("HI", timeout=5.0)


@pytest.mark.asyncio
async def test_hmp_command(qmp_bridge: QmpBridge) -> None:
    """
    Test that we can execute Human Monitor Commands (HMP).
    """
    res = await qmp_bridge.execute("human-monitor-command", {"command-line": "info version"})
    # QEMU version string can be '11.0.0' or 'v11.0.0' etc.
    assert "11.0.0" in res  # type: ignore[operator]


@pytest.mark.asyncio
async def test_uart_write(qmp_bridge: QmpBridge) -> None:
    """
    Test that we can write data to the UART.
    Since 'hello.elf' doesn't echo, we just verify the socket write doesn't raise an error.
    """
    await qmp_bridge.write_to_uart("Test string\n")
