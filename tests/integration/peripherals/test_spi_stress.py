"""
SOTA Test Module: test_spi_stress

Context:
This module implements tests for the test_spi_stress subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_spi_stress.
"""

import asyncio
import subprocess
from pathlib import Path

import pytest

from tools.testing.utils import yield_now


@pytest.mark.asyncio
async def test_spi_stress_baremetal(simulation, zenoh_session, zenoh_router, tmp_path):
    """
    Stress test for Perform 10,000 rapid SPI transactions
    through the Zenoh SPI bridge to verify backpressure, lock safety,
    and throughput stability.
    """
    from tools.testing.env import WORKSPACE_ROOT

    workspace_root = WORKSPACE_ROOT
    yaml_path = Path(workspace_root) / "tests/fixtures/guest_apps/spi_bridge/spi_test.yaml"
    dtb_path = tmp_path / "spi_stress.dtb"
    kernel_path = Path(workspace_root) / "tests/fixtures/guest_apps/spi_bridge/spi_stress.elf"

    router_endpoint = zenoh_router

    if not Path(kernel_path).exists():
        subprocess.run(["make", "-C", "tests/fixtures/guest_apps/spi_bridge"], check=True, cwd=workspace_root)

    with Path(yaml_path).open() as f:
        config = f.read()

    config = config.replace(
        "- name: spi_echo\n    type: spi-echo",
        f"- name: spi_echo\n    type: SPI.ZenohBridge\n    properties:\n      router: {router_endpoint}",
    )
    if f"router: {router_endpoint}" not in config:
        config = config.replace(
            "type: spi-echo", f"type: SPI.ZenohBridge\n    properties:\n      router: {router_endpoint}"
        )

    temp_yaml = tmp_path / "spi_stress_zenoh.yaml"
    with Path(temp_yaml).open("w") as f:
        f.write(config)

    subprocess.run(
        ["python3", "-m", "tools.yaml2qemu", str(temp_yaml), "--out-dtb", str(dtb_path)], check=True, cwd=workspace_root
    )

    topic = "sim/spi/spi0/0"

    received_queries = 0

    def on_query(query):
        nonlocal received_queries
        received_queries += 1
        payload = query.payload
        if payload:
            data_bytes = payload.to_bytes()
            if len(data_bytes) >= 28:
                data = data_bytes[24:28]
                query.reply(query.key_expr, data)

    _ = await asyncio.to_thread(lambda: zenoh_session.declare_queryable(topic, on_query))

    async with await simulation(dtb_path, kernel_path) as sim:
        # Task 20.3: In slaved-icount mode, the clock only advances when we call vta.step().
        # We need a background task to drive the simulation while we wait for UART results.
        async def drive_clock():
            try:
                while True:
                    await sim.vta.step(10_000_000)
                    await yield_now()  # Yield to allow UART processing
            except asyncio.CancelledError:
                pass

        clock_task = asyncio.create_task(drive_clock())
        try:
            success = await sim.bridge.wait_for_line_on_uart("P|F", timeout=60.0)
            if "F" in sim.bridge.uart_buffer:
                pytest.fail(f"Firmware signaled SPI stress test FAILURE. UART: {sim.bridge.uart_buffer}")

            assert success, (
                f"Firmware timed out. Received {received_queries}/10000 queries. UART: {sim.bridge.uart_buffer!r}"
            )
        finally:
            clock_task.cancel()
            await asyncio.gather(clock_task, return_exceptions=True)

        assert received_queries == 10000, f"Expected exactly 10,000 SPI transactions, got {received_queries}"
