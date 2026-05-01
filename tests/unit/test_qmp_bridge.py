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

from __future__ import annotations

import asyncio
from typing import TYPE_CHECKING
from unittest.mock import AsyncMock, patch

import pytest

if TYPE_CHECKING:
    from collections.abc import Generator
    from typing import Any


from tools.testing.qmp_bridge import QmpBridge
from tools.testing.utils import yield_now


@pytest.fixture(autouse=True)
def mock_multiplier() -> Generator[None]:
    with patch("tools.testing.qmp_bridge.get_time_multiplier", return_value=1.0):
        yield


# ── Helpers ───────────────────────────────────────────────────────────────────


def make_bridge() -> QmpBridge:
    """Return a QmpBridge with no live QMP connection."""
    return QmpBridge()


# ── get_virtual_time_ns ───────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_get_virtual_time_ns_returns_icount() -> None:
    """query-replay returning icount=42 should give 42."""
    bridge = make_bridge()
    with patch.object(bridge, "execute", new_callable=AsyncMock) as mock_execute:
        mock_execute.return_value = {"mode": "none", "icount": 42}
        result = await bridge.get_virtual_time_ns()
        assert result == 42
        mock_execute.assert_awaited_once_with("query-replay")


@pytest.mark.asyncio
async def test_get_virtual_time_ns_returns_zero_on_error() -> None:
    """Any QMP exception must be swallowed and 0 returned."""
    bridge = make_bridge()
    with patch.object(bridge, "execute", new_callable=AsyncMock) as mock_execute:
        mock_execute.side_effect = RuntimeError("QMP not ready")
        result = await bridge.get_virtual_time_ns()
        assert result == 0


@pytest.mark.asyncio
async def test_get_virtual_time_ns_returns_zero_when_icount_missing() -> None:
    """query-replay response missing 'icount' key should yield 0."""
    bridge = make_bridge()
    with patch.object(bridge, "execute", new_callable=AsyncMock) as mock_execute:
        mock_execute.return_value = {"mode": "none"}
        result = await bridge.get_virtual_time_ns()
        assert result == 0


# ── wait_for_line_on_uart ─────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_wait_for_line_immediate_match() -> None:
    """Pattern already in buffer: must return True without sleeping."""
    bridge = make_bridge()
    assert bridge is not None
    bridge.uart_buffer = "HI from firmware\n"
    with patch.object(bridge, "execute", new_callable=AsyncMock) as mock_execute:
        mock_execute.return_value = {"mode": "none", "icount": 0}
        assert bridge is not None
        result = await bridge.wait_for_line_on_uart("HI", timeout=5.0)
        assert result is True


@pytest.mark.asyncio
async def test_wait_for_line_wall_clock_timeout() -> None:
    """
    When virtual time never advances (standalone mode, icount=0), the wall-clock
    fallback must fire and return False before the test itself times out.
    """
    bridge = make_bridge()
    with patch.object(bridge, "execute", new_callable=AsyncMock) as mock_execute:
        mock_execute.return_value = {"mode": "none", "icount": 0}
        # Use a very short wall-clock timeout so the test completes quickly.
        assert bridge is not None
        result = await bridge.wait_for_line_on_uart("NEVER", timeout=0.25)
        assert result is False


@pytest.mark.asyncio
async def test_wait_for_line_virtual_time_timeout() -> None:
    """
    When virtual time IS advancing, the virtual-time branch must trigger the
    timeout (not wall-clock), returning False when icount exceeds the budget.
    """
    bridge = make_bridge()

    # Simulate icount advancing from 0 → 2_000_000_000 (2 s of virtual time)
    # on the second and subsequent calls.
    call_count = 0

    async def fake_execute(_cmd: str, _args: object = None) -> dict[str, Any]:
        nonlocal call_count
        call_count += 1
        if call_count == 1:
            return {"mode": "none", "icount": 0}  # start_vtime
        return {"mode": "none", "icount": 2_000_000_000}  # >> timeout

    with patch.object(bridge, "execute", side_effect=fake_execute):
        # timeout=1.0 s but icount jumps 2 s ahead immediately → virtual timeout
        assert bridge is not None
        result = await bridge.wait_for_line_on_uart("NEVER", timeout=1.0)
        assert result is False


@pytest.mark.asyncio
async def test_wait_for_line_match_before_timeout() -> None:
    """Pattern appears in buffer during polling: must return True."""
    bridge = make_bridge()

    async def populate_buffer() -> None:
        await yield_now()
        assert bridge is not None
        bridge.uart_buffer = "boot complete\n"

    with patch.object(bridge, "execute", new_callable=AsyncMock) as mock_execute:
        mock_execute.return_value = {"mode": "none", "icount": 0}
        asyncio.create_task(populate_buffer())  # noqa: RUF006
        assert bridge is not None
        result = await bridge.wait_for_line_on_uart("boot complete", timeout=5.0)
        assert result is True


# ── wait_for_event ────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_wait_for_event_timeout_wall_clock() -> None:
    """
    When no event arrives and vtime stays 0, the wall-clock fallback must raise
    TimeoutError within a short window.
    """
    bridge = make_bridge()
    with patch.object(bridge, "execute", new_callable=AsyncMock) as mock_execute:
        mock_execute.return_value = {"mode": "none", "icount": 0}

        import contextlib
        from collections.abc import Iterator

        @contextlib.contextmanager
        def fake_listen(*_listeners: object) -> Iterator[None]:
            yield

        bridge.qmp = type("FakeQmp", (), {"listen": fake_listen})()

        with pytest.raises(TimeoutError, match="Timed out waiting for event"):
            await bridge.wait_for_event("RESET", timeout=0.25)


@pytest.mark.asyncio
async def test_wait_for_event_virtual_time_timeout() -> None:
    """
    When virtual time advances past the budget, TimeoutError must be raised
    before the wall-clock timeout would fire.
    """
    bridge = make_bridge()

    call_count = 0

    async def fake_execute(_cmd: str, _args: object = None) -> dict[str, Any]:
        nonlocal call_count
        call_count += 1
        if call_count == 1:
            return {"mode": "none", "icount": 0}
        return {"mode": "none", "icount": 5_000_000_000}  # 5 s of virtual time

    with patch.object(bridge, "execute", side_effect=fake_execute):
        import contextlib
        from collections.abc import Iterator

        @contextlib.contextmanager
        def fake_listen(*_listeners: object) -> Iterator[None]:
            yield

        bridge.qmp = type("FakeQmp", (), {"listen": fake_listen})()

        with pytest.raises(TimeoutError, match=r".*"):
            await bridge.wait_for_event("RESET", timeout=1.0)
