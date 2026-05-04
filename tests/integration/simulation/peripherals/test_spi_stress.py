# ZENOH_HACK_EXCEPTION: SPI mock requires declare_queryable which is not in SimulationTransport
"""
SOTA Test Module: test_spi_stress

Context:
This module implements tests for the test_spi_stress subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_spi_stress.
"""

from __future__ import annotations

import asyncio
from collections.abc import Callable
from pathlib import Path
from typing import TYPE_CHECKING

import pytest

from tools.testing.virtmcu_test_suite.factory import compile_yaml
from tools.testing.virtmcu_test_suite.topics import SimTopic

if TYPE_CHECKING:
    from pathlib import Path

    import zenoh

    from tools.testing.virtmcu_test_suite.simulation import Simulation


@pytest.mark.asyncio
async def test_spi_stress_baremetal(
    simulation: Simulation, zenoh_session: zenoh.Session, zenoh_router: str, tmp_path: Path, guest_app_factory: Callable[[str], Path]
) -> None:
    """
    Stress test for Perform 10,000 rapid SPI transactions
    through the Zenoh SPI bridge to verify backpressure, lock safety,
    and throughput stability.
    """
    app_dir = guest_app_factory("spi_bridge")
    yaml_path = app_dir / "spi_test.yaml"
    kernel_path = app_dir / "spi_stress.elf"

    router_endpoint = zenoh_router

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

    dtb_path = compile_yaml(temp_yaml, tmp_path / "spi_stress.dtb")

    topic = SimTopic.spi_base("spi0", 0)

    received_queries = 0

    from tools import vproto

    def on_query(query: zenoh.Query) -> None:
        nonlocal received_queries
        received_queries += 1
        payload = query.payload
        if payload:
            data_bytes = payload.to_bytes()
            header_size = vproto.SIZE_ZENOH_SPI_HEADER
            if len(data_bytes) >= header_size + 4:
                _ = vproto.ZenohSPIHeader.unpack(data_bytes[:header_size])
                data = data_bytes[header_size : header_size + 4]
                query.reply(query.key_expr, data)

    _ = await asyncio.to_thread(lambda: zenoh_session.declare_queryable(topic, on_query))

    extra_args = ["-device", f"virtmcu-clock,node=0,router={simulation._router},mode=slaved-icount"]
    simulation.add_node(node_id=0, dtb=dtb_path, kernel=kernel_path, extra_args=extra_args)
    async with simulation as sim:
        try:
            # Wait for either completion or timeout via wait_for_uart, which drives vta automatically
            await sim.node(0).wait_for_uart("P", timeout_ns=1_000_000_000, step_ns=10_000_000)
            success = True
        except TimeoutError:
            success = False

        if "F" in sim.node(0).uart_buffer:
            pytest.fail(f"Firmware signaled SPI stress test FAILURE. UART: {sim.node(0).uart_buffer}")

        assert success, (
            f"Firmware timed out. Received {received_queries}/10000 queries. UART: {sim.node(0).uart_buffer!r}"
        )

        assert received_queries == 10000, f"Expected exactly 10,000 SPI transactions, got {received_queries}"
