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
import shutil
import subprocess
from pathlib import Path
from typing import TYPE_CHECKING

import pytest

from tools.testing.utils import yield_now
from tools.testing.virtmcu_test_suite.topics import SimTopic

if TYPE_CHECKING:
    import zenoh

    from tools.testing.virtmcu_test_suite.simulation import Simulation


logger = logging.getLogger(__name__)


@pytest.mark.asyncio
async def test_spi_echo_baremetal(
    simulation: Simulation, zenoh_session: zenoh.Session, zenoh_router: str, tmp_path: Path
) -> None:
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
        subprocess.run(
            [shutil.which("make") or "make", "-C", "tests/fixtures/guest_apps/spi_bridge"],
            check=True,
            cwd=workspace_root,
        )

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
        [shutil.which("python3") or "python3", "-m", "tools.yaml2qemu", str(temp_yaml), "--out-dtb", str(dtb_path)],
        check=True,
        cwd=workspace_root,
    )

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
        for _ in range(100):
            # Advance virtual time so firmware can run
            await sim.vta.step(1_000_000)
            # Check UART output
            assert sim.bridge is not None
            if "P" in sim.bridge.uart_buffer:
                success = True
                break
            assert sim.bridge is not None
            if "F" in sim.bridge.uart_buffer:
                assert sim.bridge is not None
                pytest.fail(f"Firmware signaled SPI verification FAILURE. UART: {sim.bridge.uart_buffer}")
            await yield_now()

        if not success:
            assert sim.bridge is not None
            logger.info(f"DEBUG: UART Buffer: {sim.bridge.uart_buffer!r}")

        assert sim.bridge is not None
        assert success, f"Firmware timed out without signaling success (P). UART: {sim.bridge.uart_buffer!r}"
