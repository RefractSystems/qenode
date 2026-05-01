"""
SOTA Test Module: test_spi

Context:
This module implements tests for the test_spi subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_spi.
"""

import asyncio
import logging
import subprocess
from pathlib import Path

import pytest

from tools.testing.utils import yield_now

logger = logging.getLogger(__name__)


@pytest.mark.asyncio
async def test_spi_echo_baremetal(simulation, zenoh_session, zenoh_router, tmp_path):
    """
    SPI Loopback/Echo Firmware.
    Verify that the ARM bare-metal firmware can perform full-duplex SPI
    transactions against a Zenoh-backed SPI bridge.
    """
    from tools.testing.env import WORKSPACE_ROOT

    workspace_root = WORKSPACE_ROOT
    yaml_path = Path(workspace_root) / "tests/fixtures/guest_apps/spi_bridge/spi_test.yaml"
    dtb_path = tmp_path / "spi_test.dtb"
    kernel_path = Path(workspace_root) / "tests/fixtures/guest_apps/spi_bridge/spi_echo.elf"

    # Get the actual router endpoint from the fixture session (simulation will provide it to QEMU)
    router_endpoint = zenoh_router

    # 1. Build firmware if missing
    if not Path(kernel_path).exists():
        subprocess.run(["make", "-C", "tests/fixtures/guest_apps/spi_bridge"], check=True, cwd=workspace_root)

    # 2. Generate DTB using yaml2qemu
    # Create a temporary yaml with Zenoh SPI Bridge
    with Path(yaml_path).open() as f:
        config = f.read()

    # Replace spi-echo with SPI.ZenohBridge and add router property
    # Target specifically the spi_echo device
    config = config.replace(
        "- name: spi_echo\n    type: spi-echo",
        f"- name: spi_echo\n    type: SPI.ZenohBridge\n    properties:\n      router: {router_endpoint}",
    )
    if f"router: {router_endpoint}" not in config:
        # Fallback
        config = config.replace(
            "type: spi-echo", f"type: SPI.ZenohBridge\n    properties:\n      router: {router_endpoint}"
        )

    temp_yaml = tmp_path / "spi_test_zenoh.yaml"
    with Path(temp_yaml).open("w") as f:
        f.write(config)

    subprocess.run(
        ["python3", "-m", "tools.yaml2qemu", str(temp_yaml), "--out-dtb", str(dtb_path)], check=True, cwd=workspace_root
    )

    # 3. Setup Zenoh Echo
    # Topic: sim/spi/{id}/{cs} -> default id is 'spi0', cs is 0
    topic = "sim/spi/spi0/0"

    def on_query(query):
        payload = query.payload
        if payload:
            data_bytes = payload.to_bytes()
            if len(data_bytes) >= 24 + 4:
                # Header is 24 bytes, data is 4 bytes
                data = data_bytes[24:28]
                # Echo back
                query.reply(query.key_expr, data)

    _ = await asyncio.to_thread(lambda: zenoh_session.declare_queryable(topic, on_query))

    # 4. Launch QEMU using VirtmcuSimulation
    async with await simulation(dtb_path, kernel_path) as sim:
        # 4. Wait for firmware to complete.
        # spi_echo.S writes 'P' (success) or 'F' (failure) to UART0.
        success = False
        for _ in range(100):
            # Advance virtual time so firmware can run
            await sim.vta.step(1_000_000)
            # Check UART output
            if "P" in sim.bridge.uart_buffer:
                success = True
                break
            if "F" in sim.bridge.uart_buffer:
                pytest.fail(f"Firmware signaled SPI verification FAILURE. UART: {sim.bridge.uart_buffer}")
            await yield_now()

        if not success:
            logger.info(f"DEBUG: UART Buffer: {sim.bridge.uart_buffer!r}")

        assert success, f"Firmware timed out without signaling success (P). UART: {sim.bridge.uart_buffer!r}"
