# ZENOH_HACK_EXCEPTION: SPI mock requires declare_queryable which is not in SimulationTransport
"""
SOTA Test Module: test_spi

Context:
This module implements tests for the test_spi subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_spi.
"""

from __future__ import annotations

import asyncio
import logging
from collections.abc import Callable
from pathlib import Path
from typing import TYPE_CHECKING

import pytest

from tools.testing.virtmcu_test_suite.factory import compile_yaml
from tools.testing.virtmcu_test_suite.topics import SimTopic

if TYPE_CHECKING:
    import zenoh

    from tools.testing.virtmcu_test_suite.simulation import Simulation


logger = logging.getLogger(__name__)


@pytest.mark.asyncio
async def test_spi_echo_baremetal(
    simulation: Simulation, zenoh_session: zenoh.Session, zenoh_router: str, tmp_path: Path, guest_app_factory: Callable[[str], Path]
) -> None:
    """
    SPI Loopback/Echo Firmware.
    Verify that the ARM bare-metal firmware can perform full-duplex SPI
    transactions against a Zenoh-backed SPI bridge.
    """
    app_dir = guest_app_factory("spi_bridge")
    yaml_path = app_dir / "spi_test.yaml"
    kernel_path = app_dir / "spi_echo.elf"

    # Get the actual router endpoint from the fixture session (simulation will provide it to QEMU)
    router_endpoint = zenoh_router

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

    dtb_path = compile_yaml(temp_yaml, tmp_path / "spi_test.dtb")

    from tools import vproto

    # 3. Setup Zenoh Echo
    # Topic: sim/spi/{id}/{cs} -> default id is 'spi0', cs is 0
    topic = SimTopic.spi_base("spi0", 0)

    def on_query(query: zenoh.Query) -> None:
        payload = query.payload
        if payload:
            data_bytes = payload.to_bytes()
            header_size = vproto.SIZE_ZENOH_SPI_HEADER
            if len(data_bytes) >= header_size + 4:
                _ = vproto.ZenohSPIHeader.unpack(data_bytes[:header_size])
                data = data_bytes[header_size : header_size + 4]
                # Echo back
                query.reply(query.key_expr, data)

    _ = await asyncio.to_thread(lambda: zenoh_session.declare_queryable(topic, on_query))

    # 4. Launch QEMU using Simulation
    simulation.add_node(node_id=0, dtb=dtb_path, kernel=kernel_path, extra_args=None)
    async with simulation as sim:
        # 4. Wait for firmware to complete.
        # spi_echo.S writes 'P' (success) or 'F' (failure) to UART0.
        success = False
        try:
            await sim.node(0).wait_for_uart("P", timeout_ns=100_000_000, step_ns=1_000_000)
            success = True
        except TimeoutError:
            success = False

        if "F" in sim.node(0).uart_buffer:
            pytest.fail(f"Firmware signaled SPI verification FAILURE. UART: {sim.node(0).uart_buffer}")

        if not success:
            logger.info(f"DEBUG: UART Buffer: {sim.node(0).uart_buffer!r}")

        assert success, f"Firmware timed out without signaling success (P). UART: {sim.node(0).uart_buffer!r}"
