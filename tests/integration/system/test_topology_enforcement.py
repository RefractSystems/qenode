"""
SOTA Test Module: test_topology_enforcement

Context:
This module implements tests for the test_topology_enforcement subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_topology_enforcement.
"""

from __future__ import annotations

import asyncio
import logging
from pathlib import Path
from typing import Any, cast

import pytest
import zenoh

from tools import vproto
from tools.testing.virtmcu_test_suite.artifact_resolver import resolve_rust_binary
from tools.testing.virtmcu_test_suite.conftest_core import coordinator_subprocess
from tools.testing.virtmcu_test_suite.constants import VirtmcuBinary
from tools.testing.virtmcu_test_suite.topics import SimTopic
from tools.testing.virtmcu_test_suite.world_schema import (
    NodeSpec,
    TopologySpec,
    WireLink,
    WorldYaml,
)

logger = logging.getLogger(__name__)


@pytest.mark.asyncio
async def test_topology_enforcement(zenoh_router: str, zenoh_session: zenoh.Session, tmp_path: Path) -> None:
    """
    Test Topology-First YAML Loading.
    The coordinator enforces the static topology and drops packets not in the graph.
    """
    coordinator_bin = resolve_rust_binary(VirtmcuBinary.DETERMINISTIC_COORDINATOR)

    world_yaml = tmp_path / "world.yaml"
    world = WorldYaml(
        topology=TopologySpec(
            nodes=[NodeSpec(name="0"), NodeSpec(name="1"), NodeSpec(name="2")],
            global_seed=42,
            links=[
                WireLink(type="uart", nodes=["0", "1"], baud=115200),
            ],
        )
    )
    world_yaml.write_text(world.to_yaml())

    # Setup listeners for rx
    received_uart_node1: list[bytes] = []
    received_eth_node2: list[bytes] = []
    rx_event = asyncio.Event()

    def on_node1_uart(sample: zenoh.Sample) -> None:
        logger.info(f"Node 1 RX: {sample.payload.to_bytes()!r}")
        received_uart_node1.append(sample.payload.to_bytes())
        rx_event.set()

    def on_node2_eth(sample: zenoh.Sample) -> None:
        logger.info(f"Node 2 RX: {sample.payload.to_bytes()!r}")
        received_eth_node2.append(sample.payload.to_bytes())
        rx_event.set()

    s = zenoh_session
    sub1 = s.declare_subscriber(SimTopic.uart_port_rx(0, 1), on_node1_uart)
    sub2 = s.declare_subscriber(SimTopic.eth_rx(2), on_node2_eth)

    async with coordinator_subprocess(
        binary=coordinator_bin,
        args=["--connect", zenoh_router, "--topology", str(world_yaml), "--nodes", "3", "--no-pdes"],
        zenoh_session=s,
    ):
        # 1. Send UART from 0 to 1 (ALLOWED) repeatedly until it arrives (handles discovery delay)
        pub_uart_tx0 = s.declare_publisher(SimTopic.uart_port_tx(0, 0))
        msg = b"HELLO"
        header = vproto.ZenohFrameHeader(1000, 0, len(msg)).pack()

        while not rx_event.is_set():
            pub_uart_tx0.put(header + msg)
            try:
                await asyncio.wait_for(rx_event.wait(), timeout=0.1)
            except TimeoutError:
                pass

        assert len(received_uart_node1) > 0
        assert msg in received_uart_node1[-1]

        rx_event.clear()

        # 2. Send ETH from 0 to 2 (BANNED - not in links)
        pub_eth_tx0 = s.declare_publisher(SimTopic.eth_tx(0))

        # 3. Send PROBE to ensure processing. Loop in case of further discovery delays
        # or Zenoh topic ordering races.
        while True:
            pub_eth_tx0.put(header + b"BANNED")
            pub_uart_tx0.put(header + b"PROBE")
            try:
                await asyncio.wait_for(rx_event.wait(), timeout=1.0)
                if any(b"PROBE" in m for m in received_uart_node1):
                    break
                rx_event.clear()
            except TimeoutError:
                pass

        assert len(received_eth_node2) == 0, "BANNED ETH message was forwarded"

    cast(Any, sub1).undeclare()
    cast(Any, sub2).undeclare()


@pytest.fixture(autouse=True)
def _noop_fixture() -> None:
    # Keeps autouse pattern but does nothing as we inlined world.yaml
    pass
