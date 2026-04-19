import asyncio
import logging
import time

import pytest

logger = logging.getLogger(__name__)

@pytest.mark.asyncio
async def test_qmp_rapid_commands(qmp_bridge):
    """
    Stress test: Send 100 QMP commands in rapid succession.
    """
    start_time = time.time()
    for i in range(100):
        res = await qmp_bridge.execute("query-status")
        assert "running" in res
    duration = time.time() - start_time
    logger.info(f"100 query-status commands took {duration:.2f}s ({(100/duration):.2f} cmd/s)")

@pytest.mark.asyncio
async def test_qmp_concurrent_commands(qmp_bridge):
    """
    Stress test: Send multiple QMP commands concurrently.
    """
    async def task(id):
        res = await qmp_bridge.execute("query-version")
        assert "qemu" in res
        return id

    tasks = [task(i) for i in range(50)]
    results = await asyncio.gather(*tasks)
    assert len(results) == 50
    assert set(results) == set(range(50))

@pytest.mark.asyncio
async def test_uart_throughput(qmp_bridge):
    """
    Stress test: Verify UART buffer can handle sustained output if QEMU keeps printing.
    (Note: This depends on the firmware behavior. Phase 1 hello.elf prints once.)
    For this test, we'll just check if multiple reads don't break anything.
    """
    for _ in range(10):
        await qmp_bridge.wait_for_line_on_uart("HI", timeout=1.0)
        # clear and wait again? hello.elf only prints once.
        # So we just verify we can still talk to it.
        res = await qmp_bridge.execute("query-status")
        assert res["running"] is True

@pytest.mark.asyncio
async def test_pc_polling_stress(qmp_bridge):
    """
    Stress test: Poll PC 100 times.
    This also tests HMP command overhead.
    """
    pcs = []
    for _ in range(100):
        pc = await qmp_bridge.get_pc()
        pcs.append(pc)

    assert len(pcs) == 100
    # PC might or might not change depending on if it's in a loop
    logger.info(f"Sampled 100 PCs, first: {hex(pcs[0])}, last: {hex(pcs[-1])}")
