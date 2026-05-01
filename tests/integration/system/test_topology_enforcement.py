"""
SOTA Test Module: test_topology

Context:
This module implements tests for the test_topology subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_topology.
"""

import asyncio
import logging
from pathlib import Path

import pytest
import yaml

from tools.testing.virtmcu_test_suite.artifact_resolver import resolve_rust_binary

logger = logging.getLogger(__name__)


@pytest.mark.asyncio
async def test_topology_enforcement(zenoh_router, zenoh_session, tmp_path):
    """
    Test Topology-First YAML Loading.
    The coordinator enforces the static topology and drops packets not in the graph.
    """
    coordinator_bin = resolve_rust_binary("deterministic_coordinator")

    # 1. Create a world YAML with topology
    world_yaml = tmp_path / "world.yaml"
    topology = {
        "nodes": [{"id": "0"}, {"id": "1"}, {"id": "2"}],
        "topology": {
            "global_seed": 42,
            "transport": "zenoh",
            "links": [{"type": "uart", "nodes": ["0", "1"], "baud": 115200}],
        },
    }
    with Path(world_yaml).open("w") as f:
        yaml.dump(topology, f)

    # 2. Start coordinator with --topology
    from tools.testing.virtmcu_test_suite.process import AsyncManagedProcess

    async with AsyncManagedProcess(
        "stdbuf",
        "-oL",
        str(coordinator_bin),
        "--connect",
        zenoh_router,
        "--topology",
        str(world_yaml),
        "--nodes",
        "3",
    ) as proc:
        try:
            await proc.wait_for_line("Coordinator subscriber active", target="stderr", timeout=10.0)

            # Setup listeners for rx
            received_uart_node1 = []
            received_eth_node2 = []
            rx_event = asyncio.Event()

            def on_uart_rx(sample):
                # parse the CoordMessage
                received_uart_node1.append(sample.payload.to_bytes())
                rx_event.set()

            def on_eth_rx(sample):
                received_eth_node2.append(sample.payload.to_bytes())
                rx_event.set()

            _sub1 = await asyncio.to_thread(lambda: zenoh_session.declare_subscriber("sim/coord/1/rx", on_uart_rx))
            _sub2 = await asyncio.to_thread(lambda: zenoh_session.declare_subscriber("sim/coord/2/rx", on_eth_rx))

            # 4. Send an Ethernet frame from node 0 to node 2 (not in the graph) via Zenoh
            # Construct CoordMessage
            vtime = 0
            msg_payload_eth = b"WORLD"
            msg_eth = (
                (1).to_bytes(4, "little")
                + (0).to_bytes(4, "little")
                + (2).to_bytes(4, "little")
                + vtime.to_bytes(8, "little")
                + (1).to_bytes(8, "little")
                + (0).to_bytes(1, "little")
                + len(msg_payload_eth).to_bytes(4, "little")
                + msg_payload_eth
            )

            # Send UART from 0 to 1 (in the graph)
            msg_payload_uart = b"HELLO"
            msg_uart = (
                (1).to_bytes(4, "little")
                + (0).to_bytes(4, "little")
                + (1).to_bytes(4, "little")
                + vtime.to_bytes(8, "little")
                + (2).to_bytes(8, "little")
                + (1).to_bytes(1, "little")
                + len(msg_payload_uart).to_bytes(4, "little")
                + msg_payload_uart
            )

            def _send():
                zenoh_session.put("sim/coord/0/tx", msg_eth)
                zenoh_session.put("sim/coord/0/tx", msg_uart)
                # Send done signals to advance barrier
                # quantum number starts at 1
                zenoh_session.put("sim/coord/0/done", (1).to_bytes(8, "little"))
                zenoh_session.put("sim/coord/1/done", (1).to_bytes(8, "little"))
                zenoh_session.put("sim/coord/2/done", (1).to_bytes(8, "little"))

            # Send messages on background thread to avoid blocking loop
            await asyncio.to_thread(_send)

            # Wait for message reception or timeout using event signaling
            try:
                await asyncio.wait_for(rx_event.wait(), timeout=2.0)
                success = True
            except TimeoutError:
                success = False

            assert success, "Zenoh delivery timed out"

            # Assertions
            assert len(received_uart_node1) > 0, "UART message 0->1 should have been delivered"
            # Node 1 Rx check
            payload = received_uart_node1[0]
            assert b"HELLO" in payload

            # Node 2 Eth Rx check
            assert len(received_eth_node2) == 0, "ETH message 0->2 should have been blocked"

        finally:
            pass

    # Outside of context manager the process is terminated
    stdout = proc.stdout_text
    stderr = proc.stderr_text

    # Assert the coordinator log contains a topology violation entry for this message.
    assert "Topology violation: dropped" in stderr or "Topology violation: dropped" in stdout
