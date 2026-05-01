"""
SOTA Test Module: test_wireless

Context:
This module implements tests for the test_wireless subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_wireless.
"""

import asyncio
import logging
from pathlib import Path

import pytest
import yaml

from tools.testing.virtmcu_test_suite.artifact_resolver import resolve_rust_binary

logger = logging.getLogger(__name__)


@pytest.mark.asyncio
async def test_wireless_topology(zenoh_router, zenoh_session, tmp_path):
    """
    Test Wireless Topology Enforcement.
    The coordinator delivers wireless messages based on distance.
    """
    coordinator_bin = resolve_rust_binary("deterministic_coordinator")

    # 1. Create a world YAML with wireless topology
    # Node 0 at (0,0,0)
    # Node 1 at (5,0,0) - In range (max_range=10)
    # Node 2 at (15,0,0) - Out of range
    world_yaml = tmp_path / "world.yaml"
    topology = {
        "nodes": [{"id": "0"}, {"id": "1"}, {"id": "2"}],
        "topology": {
            "global_seed": 42,
            "transport": "zenoh",
            "wireless": {
                "medium": "ieee802154",
                "max_range_m": 10.0,
                "nodes": [
                    {"id": "0", "initial_position": [0.0, 0.0, 0.0]},
                    {"id": "1", "initial_position": [5.0, 0.0, 0.0]},
                    {"id": "2", "initial_position": [15.0, 0.0, 0.0]},
                ],
            },
        },
    }
    with Path(world_yaml).open("w") as f:
        yaml.dump(topology, f)

    # 2. Start coordinator
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
            received_node1 = []
            received_node2 = []
            rx_event = asyncio.Event()

            def on_rx_node1(sample):
                received_node1.append(sample.payload.to_bytes())
                rx_event.set()

            def on_rx_node2(sample):
                received_node2.append(sample.payload.to_bytes())
                rx_event.set()

            _sub1 = await asyncio.to_thread(lambda: zenoh_session.declare_subscriber("sim/coord/1/rx", on_rx_node1))
            _sub2 = await asyncio.to_thread(lambda: zenoh_session.declare_subscriber("sim/coord/2/rx", on_rx_node2))

            # 4. Send a wireless BROADCAST from node 0
            # Protocol 6 = Rf802154
            vtime = 1000
            msg_payload = b"BEACON"
            msg_broadcast = (
                (1).to_bytes(4, "little")
                + (0).to_bytes(4, "little")
                + (0xFFFFFFFF).to_bytes(4, "little")
                + vtime.to_bytes(8, "little")
                + (1).to_bytes(8, "little")
                + (6).to_bytes(1, "little")
                + len(msg_payload).to_bytes(4, "little")
                + msg_payload
            )

            def _send():
                zenoh_session.put("sim/coord/0/tx", msg_broadcast)
                # Send done signals to advance barrier
                zenoh_session.put("sim/coord/0/done", (1).to_bytes(8, "little"))
                zenoh_session.put("sim/coord/1/done", (1).to_bytes(8, "little"))
                zenoh_session.put("sim/coord/2/done", (1).to_bytes(8, "little"))

            await asyncio.to_thread(_send)

            # Wait for message reception on both node 1 and 2
            success = False
            try:
                await asyncio.wait_for(rx_event.wait(), timeout=2.0)
                success = True
            except TimeoutError:
                success = False

            if not success:
                logger.error(f"STDOUT: {proc.stdout_text}")
                logger.error(f"STDERR: {proc.stderr_text}")

            assert success, "Broadcast not delivered to node 1"
            assert len(received_node1) == 1
            assert len(received_node2) == 0, "Broadcast delivered to out-of-range node 2"

            # The coordinator should have rewritten the dst_node_id to 1
            # Unpack without num_msgs: src(I), dst(I), vtime(Q), seq(Q), proto(B), len(I) = 29 bytes
            dst = int.from_bytes(received_node1[0][4:8], "little")
            assert dst == 1, f"Expected dst_node_id=1, got {dst}"
            assert received_node1[0][29:] == b"BEACON"

        finally:
            pass
