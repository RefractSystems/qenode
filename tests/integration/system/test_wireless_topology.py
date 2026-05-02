"""
SOTA Test Module: test_wireless_topology

Context:
This module implements tests for the test_wireless_topology subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_wireless_topology.
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
    WirelessMedium,
    WirelessNode,
    WorldYaml,
)

logger = logging.getLogger(__name__)


@pytest.mark.asyncio
async def test_wireless_topology(zenoh_router: str, zenoh_session: zenoh.Session, tmp_path: Path) -> None:
    """
    Test Wireless Topology Enforcement.
    The coordinator delivers wireless messages based on distance.
    """
    coordinator_bin = resolve_rust_binary(VirtmcuBinary.DETERMINISTIC_COORDINATOR)

    world_yaml = tmp_path / "world.yaml"
    world = WorldYaml(
        topology=TopologySpec(
            nodes=[NodeSpec(name="0"), NodeSpec(name="1"), NodeSpec(name="2")],
            global_seed=42,
            wireless=WirelessMedium(
                medium="ieee802154",
                max_range_m=10.0,
                nodes=[
                    WirelessNode(name="0", initial_position=[0.0, 0.0, 0.0]),
                    WirelessNode(name="1", initial_position=[5.0, 0.0, 0.0]),
                    WirelessNode(name="2", initial_position=[15.0, 0.0, 0.0]),
                ],
            ),
        )
    )
    world_yaml.write_text(world.to_yaml())

    received_node1: list[bytes] = []
    received_node2: list[bytes] = []
    rx_event = asyncio.Event()

    def on_node1_rx(sample: zenoh.Sample) -> None:
        logger.info(f"Node 1 RX: {sample.payload.to_bytes()!r}")
        received_node1.append(sample.payload.to_bytes())
        rx_event.set()

    def on_node2_rx(sample: zenoh.Sample) -> None:
        logger.info(f"Node 2 RX: {sample.payload.to_bytes()!r}")
        received_node2.append(sample.payload.to_bytes())
        rx_event.set()

    s = zenoh_session
    sub1 = s.declare_subscriber(SimTopic.rf_ieee802154_rx(1), on_node1_rx)
    sub2 = s.declare_subscriber(SimTopic.rf_ieee802154_rx(2), on_node2_rx)

    async with coordinator_subprocess(
        binary=coordinator_bin,
        args=["--connect", zenoh_router, "--topology", str(world_yaml), "--nodes", "3", "--no-pdes"],
        zenoh_session=s,
    ):
        # 1. Send Wireless from 0 (at 0,0,0) repeatedly until it arrives (handles discovery delay)
        pub_rf_tx0 = s.declare_publisher(SimTopic.rf_ieee802154_tx(0))
        msg = b"WIRELESS"
        # We use ZenohFrameHeader as a fallback, which the coordinator should now support for RF too.
        header = vproto.ZenohFrameHeader(1000, 0, len(msg)).pack()

        while not rx_event.is_set():
            pub_rf_tx0.put(header + msg)
            try:
                await asyncio.wait_for(rx_event.wait(), timeout=0.1)
            except TimeoutError:
                pass

        assert len(received_node1) > 0
        assert len(received_node2) == 0, "Wireless message delivered to Node 2 (out of range)"

    cast(Any, sub1).undeclare()
    cast(Any, sub2).undeclare()


@pytest.fixture(autouse=True)
def _noop_fixture() -> None:
    pass
