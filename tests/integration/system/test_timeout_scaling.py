"""
SOTA Test Module: test_timeout

Context:
This module implements tests for the test_timeout subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_timeout.
"""

import os
from unittest.mock import AsyncMock, MagicMock, patch

import pytest

from tools.testing.utils import get_time_multiplier


def test_get_time_multiplier_scaling():
    # Test local / unscaled
    with patch.dict(os.environ, {}, clear=True):
        assert get_time_multiplier() == 1.0

    # Test CI
    with patch.dict(os.environ, {"CI": "true"}, clear=True):
        assert get_time_multiplier() == 2.0

    # Test ASan
    with patch.dict(os.environ, {"VIRTMCU_USE_ASAN": "1"}, clear=True):
        assert get_time_multiplier() == 5.0

    # Test TSan
    with patch.dict(os.environ, {"VIRTMCU_USE_TSAN": "1"}, clear=True):
        assert get_time_multiplier() == 10.0


@pytest.mark.asyncio
async def test_qemu_launcher_injects_stall_timeout(qemu_launcher):
    """
    Assert that QEMU's stall-timeout parameter is dynamically multiplied
    via qemu_launcher before QEMU instantiation.
    """
    from tools.testing.virtmcu_test_suite.conftest_core import _stall_timeout_ms

    with (
        patch("asyncio.create_subprocess_exec", new_callable=AsyncMock) as mock_exec,
        patch("tools.testing.virtmcu_test_suite.conftest_core.wait_for_file_creation", new_callable=AsyncMock),
        patch("tools.testing.qmp_bridge.QmpBridge.connect", new_callable=AsyncMock),
    ):
        # create a dummy process mock
        mock_proc_obj = MagicMock()
        mock_proc_obj.pid = 1234

        # mock streams
        mock_stream = AsyncMock()
        mock_stream.readline.return_value = b""  # EOF
        mock_proc_obj.stdout = mock_stream
        mock_proc_obj.stderr = mock_stream

        mock_proc_obj.returncode = None
        import asyncio
        exit_event = asyncio.Event()
        mock_proc_obj.wait = AsyncMock(side_effect=exit_event.wait)
        mock_proc_obj.terminate = exit_event.set

        mock_exec.return_value = mock_proc_obj

        # We must use ignore_clock_check=True because we're passing virtmcu-clock directly
        await qemu_launcher(
            "tests/fixtures/guest_apps/boot_arm/minimal.dtb",
            extra_args=["-device", "virtmcu-clock"],
            ignore_clock_check=True,
        )

        # Check what was passed to create_subprocess_exec
        assert mock_exec.called
        args, _kwargs = mock_exec.call_args

        # Verify the stall-timeout was injected into the clock device arg
        found_clock = False
        for arg in args:
            if "virtmcu-clock" in arg:
                found_clock = True
                assert f"stall-timeout={_stall_timeout_ms}" in arg

        assert found_clock, "virtmcu-clock argument was not found in QEMU command line"


@pytest.mark.asyncio
async def test_wait_for_line_scales_timeout():
    """
    Verify that tests using `timeout=2.0` automatically wait `10.0` seconds under ASan.
    We test this by mocking loop.time() and verifying the time delta checked within wait_for_line_on_uart.
    """

    from tools.testing.qmp_bridge import QmpBridge

    with (
        patch("tools.testing.qmp_bridge.get_time_multiplier", return_value=5.0),
        patch("asyncio.get_running_loop") as mock_get_loop,
    ):
        # We want to simulate a timeout.
        # It loops and checks loop.time() - start_wall_time > timeout
        mock_loop = MagicMock()
        mock_get_loop.return_value = mock_loop

        # Make time advance progressively
        times = [0.0, 5.0, 9.9, 10.1]
        mock_loop.time.side_effect = lambda: times.pop(0) if times else 20.0

        bridge = QmpBridge()
        bridge.execute = AsyncMock(return_value={"mode": "none", "icount": 0})  # type: ignore[method-assign]
        # Mock wait_for to just immediately raise TimeoutError to simulate 0.1s yielding
        with patch("asyncio.wait_for") as mock_wait:

            async def fake_wait_for(aw, *args, **kwargs):
                _, _ = args, kwargs
                aw.close()  # prevent unawaited coroutine warning
                raise TimeoutError()

            mock_wait.side_effect = fake_wait_for

            # This should timeout and return False after loop.time() exceeds 10.0
            result = await bridge.wait_for_line_on_uart("NEVER_APPEARS", timeout=2.0)

            assert result is False
            # If it scaled 2.0 * 5.0 = 10.0, it had to call time() enough times to pass 10.1
            assert len(times) == 0, "Did not loop enough times to reach 10.1 seconds"
