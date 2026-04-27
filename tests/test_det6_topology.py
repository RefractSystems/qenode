import asyncio
import os
import struct
from pathlib import Path

import pytest
import yaml


@pytest.mark.asyncio
async def test_det6_topology_enforcement(zenoh_router, zenoh_session, tmp_path):
    """
    Test DET-6: Topology-First YAML Loading.
    The coordinator should enforce the static topology and drop packets not in the graph.
    """
    workspace_root = Path(__file__).parent.parent
    coordinator_bin = Path(os.environ.get("CARGO_TARGET_DIR", workspace_root / "target")) / "release/zenoh_coordinator"
    if not coordinator_bin.exists():
        coordinator_bin = workspace_root / "tools/zenoh_coordinator/target/release/zenoh_coordinator"
    if not coordinator_bin.exists():
        pytest.skip("zenoh_coordinator binary not found")

    # 1. Create a world YAML with topology
    world_yaml = tmp_path / "world.yaml"
    topology = {
        "nodes": [{"id": "0"}, {"id": "1"}, {"id": "2"}],
        "topology": {
            "global_seed": 42,
            "links": [
                {
                    "type": "uart",
                    "nodes": ["0", "1"]
                }
            ]
        }
    }
    with Path(world_yaml).open("w") as f:
        yaml.dump(topology, f)

    # 2. Start coordinator with --topology
    proc = await asyncio.create_subprocess_exec(
        str(coordinator_bin),
        "--connect", zenoh_router,
        "--topology", str(world_yaml),
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
    )

    try:
        await asyncio.sleep(1.0) # wait for startup

        # 3. Setup listeners for rx
        received_uart_node1 = []
        received_eth_node2 = []

        def on_uart_rx(sample):
            received_uart_node1.append(sample.payload.to_bytes())

        def on_eth_rx(sample):
            received_eth_node2.append(sample.payload.to_bytes())

        _sub1 = await asyncio.to_thread(lambda: zenoh_session.declare_subscriber("virtmcu/uart/1/rx", on_uart_rx))
        _sub2 = await asyncio.to_thread(lambda: zenoh_session.declare_subscriber("sim/eth/frame/2/rx", on_eth_rx))
        await asyncio.sleep(0.5)

        # Send a registration message so coordinator knows the nodes
        def _reg():
            zenoh_session.put("virtmcu/uart/0/tx", struct.pack("<QQI", 0, 0, 1) + b"A")
            zenoh_session.put("sim/eth/frame/0/tx", struct.pack("<QQI", 0, 0, 1) + b"A")
            zenoh_session.put("virtmcu/uart/1/tx", struct.pack("<QQI", 0, 0, 1) + b"A")
            zenoh_session.put("sim/eth/frame/2/tx", struct.pack("<QQI", 0, 0, 1) + b"A")

        await asyncio.to_thread(_reg)
        await asyncio.sleep(0.5)

        # 4. Send UART from 0 -> 1 (allowed by topology)
        def _send_allowed():
            payload = struct.pack("<QQI", 1000, 0, 5) + b"HELLO"
            zenoh_session.put("virtmcu/uart/0/tx", payload)

        await asyncio.to_thread(_send_allowed)

        # 5. Send ETH from 0 -> 2 (NOT allowed by topology)
        def _send_blocked():
            payload = struct.pack("<QQI", 1000, 0, 5) + b"WORLD"
            zenoh_session.put("sim/eth/frame/0/tx", payload)
        await asyncio.to_thread(_send_blocked)

        await asyncio.sleep(1.0)

        # Assertions
        assert len(received_uart_node1) > 0, "UART message 0->1 should have been delivered"
        # The registration message (size 1) might be received, but the "WORLD" message (size 5) should be blocked.
        eth_payloads = [p[12:] for p in received_eth_node2]
        assert b"WORLD" not in eth_payloads, "ETH message 0->2 should have been blocked"

    finally:
        proc.terminate()
        await proc.wait()

        # Check stderr for topology violation log
        assert proc.stderr is not None
        assert proc.stdout is not None
        stderr = await proc.stderr.read()
        stderr_str = stderr.decode()
        stdout_str = (await proc.stdout.read()).decode()
        print("STDOUT:", stdout_str)
        print("STDERR:", stderr_str)

        assert "[Topology Violation] Dropping ETH msg from 0 to 2" in stderr_str
