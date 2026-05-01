"""
SOTA Test Module: test_qmp_stress

Context:
This module implements tests for the test_qmp_stress subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_qmp_stress.
"""

from __future__ import annotations

import asyncio
import logging
import os
import time
from typing import TYPE_CHECKING

import pytest

if TYPE_CHECKING:
    from tools.testing.qmp_bridge import QmpBridge


logger = logging.getLogger(__name__)


@pytest.mark.asyncio
async def test_qmp_rapid_commands(qmp_bridge: QmpBridge) -> None:
    """
    Stress test: Send 100 QMP commands in rapid succession.
    """
    start_time = time.time()
    for _i in range(100):
        res = await qmp_bridge.execute("query-status")
        assert "running" in res  # type: ignore[operator]
    duration = time.time() - start_time
    logger.info(f"100 query-status commands took {duration:.2f}s ({(100 / duration):.2f} cmd/s)")


@pytest.mark.asyncio
async def test_qmp_concurrent_commands(qmp_bridge: QmpBridge) -> None:
    """
    Stress test: Send multiple QMP commands concurrently.
    """

    async def task(task_id: int) -> int:
        res = await qmp_bridge.execute("query-version")
        assert "qemu" in res  # type: ignore[operator]
        return task_id

    iters = 10 if os.environ.get("VIRTMCU_USE_ASAN") == "1" else 50
    tasks = [task(i) for i in range(iters)]
    results = await asyncio.gather(*tasks)
    assert len(results) == iters
    assert set(results) == set(range(iters))


@pytest.mark.asyncio
async def test_uart_throughput(qmp_bridge: QmpBridge) -> None:
    """
    Stress test: Verify UART buffer can handle sustained output if QEMU keeps printing.
    (Note: This depends on the firmware behavior. hello.elf prints once.)
    For this test, we'll just check if multiple reads don't break anything.
    """
    for _ in range(10):
        assert qmp_bridge is not None
        await qmp_bridge.wait_for_line_on_uart("HI", timeout=1.0)
        # clear and wait again? hello.elf only prints once.
        # So we just verify we can still talk to it.
        res = await qmp_bridge.execute("query-status")
        assert res["running"] is True  # type: ignore[index]


@pytest.mark.asyncio
async def test_pc_polling_stress(qmp_bridge: QmpBridge) -> None:
    """
    Stress test: Poll PC 100 times.
    This also tests HMP command overhead.
    """
    pcs = []
    iters = 10 if os.environ.get("VIRTMCU_USE_ASAN") == "1" else 100
    for _ in range(iters):
        pc = await qmp_bridge.get_pc()
        pcs.append(pc)

    assert len(pcs) == 100
    # PC might or might not change depending on if it's in a loop
    logger.info(f"Sampled 100 PCs, first: {hex(pcs[0])}, last: {hex(pcs[-1])}")
