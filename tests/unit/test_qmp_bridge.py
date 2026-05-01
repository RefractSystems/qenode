"""
tests/test_qmp_bridge.py — Unit tests for tools/testing/qmp_bridge.py

Tests the virtual-time-aware timeout logic in isolation (no QEMU binary needed).
QmpBridge.execute() is patched so QMP round-trips are never made.

Scenarios covered:
  - get_virtual_time_ns: returns icount on success, 0 on QMP error
  - wait_for_line_on_uart: pattern already in buffer, wall-clock timeout
    fallback (vtime == 0), virtual-time timeout (vtime advancing), match before
    timeout
  - wait_for_event: timeout path (no event arrives, vtime static)
"""

import asyncio
import sys
from pathlib import Path
from unittest.mock import AsyncMock, patch

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent / ".."))
from tools.testing.qmp_bridge import QmpBridge
from tools.testing.utils import yield_now


@pytest.fixture(autouse=True)
def mock_multiplier():
    with patch("tools.testing.qmp_bridge.get_time_multiplier", return_value=1.0):
        yield


# ── Helpers ───────────────────────────────────────────────────────────────────


def make_bridge() -> QmpBridge:
    """Return a QmpBridge with no live QMP connection."""
    return QmpBridge()


# ── get_virtual_time_ns ───────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_get_virtual_time_ns_returns_icount():
    """query-replay returning icount=42 should give 42."""
    bridge = make_bridge()
    bridge.execute = AsyncMock(return_value={"mode": "none", "icount": 42})  # type: ignore[method-assign]
    result = await bridge.get_virtual_time_ns()
    assert result == 42
    bridge.execute.assert_awaited_once_with("query-replay")


@pytest.mark.asyncio
async def test_get_virtual_time_ns_returns_zero_on_error():
    """Any QMP exception must be swallowed and 0 returned."""
    bridge = make_bridge()
    bridge.execute = AsyncMock(side_effect=RuntimeError("QMP not ready"))  # type: ignore[method-assign]
    result = await bridge.get_virtual_time_ns()
    assert result == 0


@pytest.mark.asyncio
async def test_get_virtual_time_ns_returns_zero_when_icount_missing():
    """query-replay response missing 'icount' key should yield 0."""
    bridge = make_bridge()
    bridge.execute = AsyncMock(return_value={"mode": "none"})  # type: ignore[method-assign]
    result = await bridge.get_virtual_time_ns()
    assert result == 0


# ── wait_for_line_on_uart ─────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_wait_for_line_immediate_match():
    """Pattern already in buffer: must return True without sleeping."""
    bridge = make_bridge()
    bridge.uart_buffer = "HI from firmware\n"
    bridge.execute = AsyncMock(return_value={"mode": "none", "icount": 0})  # type: ignore[method-assign]
    result = await bridge.wait_for_line_on_uart("HI", timeout=5.0)
    assert result is True


@pytest.mark.asyncio
async def test_wait_for_line_wall_clock_timeout():
    """
    When virtual time never advances (standalone mode, icount=0), the wall-clock
    fallback must fire and return False before the test itself times out.
    """
    bridge = make_bridge()
    bridge.execute = AsyncMock(return_value={"mode": "none", "icount": 0})  # type: ignore[method-assign]
    # Use a very short wall-clock timeout so the test completes quickly.
    result = await bridge.wait_for_line_on_uart("NEVER", timeout=0.25)
    assert result is False


@pytest.mark.asyncio
async def test_wait_for_line_virtual_time_timeout():
    """
    When virtual time IS advancing, the virtual-time branch must trigger the
    timeout (not wall-clock), returning False when icount exceeds the budget.
    """
    bridge = make_bridge()

    # Simulate icount advancing from 0 → 2_000_000_000 (2 s of virtual time)
    # on the second and subsequent calls.
    call_count = 0

    async def fake_execute(cmd, args=None):  # noqa: ARG001
        nonlocal call_count
        call_count += 1
        if call_count == 1:
            return {"mode": "none", "icount": 0}  # start_vtime
        return {"mode": "none", "icount": 2_000_000_000}  # >> timeout

    bridge.execute = fake_execute  # type: ignore[method-assign]
    # timeout=1.0 s but icount jumps 2 s ahead immediately → virtual timeout
    result = await bridge.wait_for_line_on_uart("NEVER", timeout=1.0)
    assert result is False


@pytest.mark.asyncio
async def test_wait_for_line_match_before_timeout():
    """Pattern appears in buffer during polling: must return True."""
    bridge = make_bridge()
    bridge.execute = AsyncMock(return_value={"mode": "none", "icount": 0})  # type: ignore[method-assign]

    async def populate_buffer():
        await yield_now()
        bridge.uart_buffer = "boot complete\n"

    asyncio.create_task(populate_buffer())  # noqa: RUF006
    result = await bridge.wait_for_line_on_uart("boot complete", timeout=5.0)
    assert result is True


# ── wait_for_event ────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_wait_for_event_timeout_wall_clock():
    """
    When no event arrives and vtime stays 0, the wall-clock fallback must raise
    TimeoutError within a short window.
    """
    bridge = make_bridge()
    bridge.execute = AsyncMock(return_value={"mode": "none", "icount": 0})  # type: ignore[method-assign]

    # Patch qmp.listen() to produce an async generator that never yields.
    import contextlib

    @contextlib.contextmanager
    def fake_listen(*listeners):  # noqa: ARG001
        yield

    bridge.qmp = type("FakeQmp", (), {"listen": fake_listen})()

    with pytest.raises(TimeoutError, match="Timed out waiting for event"):
        await bridge.wait_for_event("RESET", timeout=0.25)


@pytest.mark.asyncio
async def test_wait_for_event_virtual_time_timeout():
    """
    When virtual time advances past the budget, TimeoutError must be raised
    before the wall-clock timeout would fire.
    """
    bridge = make_bridge()

    call_count = 0

    async def fake_execute(cmd, args=None):  # noqa: ARG001
        nonlocal call_count
        call_count += 1
        if call_count == 1:
            return {"mode": "none", "icount": 0}
        return {"mode": "none", "icount": 5_000_000_000}  # 5 s of virtual time

    bridge.execute = fake_execute  # type: ignore[method-assign]

    import contextlib

    @contextlib.contextmanager
    def fake_listen(*listeners):  # noqa: ARG001
        yield

    bridge.qmp = type("FakeQmp", (), {"listen": fake_listen})()

    with pytest.raises(TimeoutError):
        await bridge.wait_for_event("RESET", timeout=1.0)
