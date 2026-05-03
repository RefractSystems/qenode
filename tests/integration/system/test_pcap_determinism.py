"""
SOTA Test Module: test_pcap_determinism

Context:
This module implements tests for the test_pcap_determinism subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_pcap_determinism.
"""

from __future__ import annotations

import asyncio
import logging
from typing import TYPE_CHECKING, Any, cast

import pytest

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

if TYPE_CHECKING:
    from pathlib import Path

    import zenoh


logger = logging.getLogger(__name__)


@pytest.mark.asyncio
async def test_pcap_determinism(zenoh_router: str, zenoh_session: zenoh.Session, tmp_path: Path) -> None:
    coordinator_bin = resolve_rust_binary(VirtmcuBinary.DETERMINISTIC_COORDINATOR)

    world_yaml = tmp_path / "world.yaml"
    world = WorldYaml(
        topology=TopologySpec(
            nodes=[NodeSpec(name="0"), NodeSpec(name="1")],
            global_seed=42,
            links=[
                WireLink(type="uart", nodes=["0", "1"]),
                WireLink(type="ethernet", nodes=["0", "1"]),
            ],
        )
    )
    world_yaml.write_text(world.to_yaml())

    async def run_simulation(pcap_path: Path) -> None:
        msg_eth = vproto.CoordMessage(
            src_node_id=0,
            dst_node_id=1,
            delivery_vtime_ns=1000,
            sequence_number=1,
            protocol=0,  # Ethernet
            payload=b"ETH",
        )
        msg_uart1 = vproto.CoordMessage(
            src_node_id=0,
            dst_node_id=1,
            delivery_vtime_ns=2000,
            sequence_number=2,
            protocol=1,  # Uart
            payload=b"UART1",
        )
        msg_uart2 = vproto.CoordMessage(
            src_node_id=1,
            dst_node_id=0,
            delivery_vtime_ns=3000,
            sequence_number=3,
            protocol=1,  # Uart
            payload=b"UART2",
        )
        loop = asyncio.get_running_loop()
        quantum_event = asyncio.Event()

        def on_start(sample: object) -> None:
            q = int.from_bytes(cast(Any, sample).payload.to_bytes(), "little")
            if q == 2:
                loop.call_soon_threadsafe(quantum_event.set)

        sub = await asyncio.to_thread(lambda: zenoh_session.declare_subscriber(SimTopic.clock_start(0), on_start))

        rx_received: set[str] = set()
        rx_event = asyncio.Event()

        def on_rx(sample: zenoh.Sample) -> None:
            payload = sample.payload.to_bytes()
            if b"ETH" in payload:
                rx_received.add("ETH")
            if b"UART1" in payload:
                rx_received.add("UART1")
            if b"UART2" in payload:
                rx_received.add("UART2")
            if len(rx_received) == 3:
                loop.call_soon_threadsafe(rx_event.set)

        sub_eth = await asyncio.to_thread(
            lambda: zenoh_session.declare_subscriber(SimTopic.ETH_FRAME_RX_WILDCARD, on_rx)
        )
        sub_uart = await asyncio.to_thread(
            lambda: zenoh_session.declare_subscriber(SimTopic.SIM_UART_RX_WILDCARD, on_rx)
        )

        pub0 = await asyncio.to_thread(lambda: zenoh_session.declare_publisher(SimTopic.coord_done(0)))
        pub1 = await asyncio.to_thread(lambda: zenoh_session.declare_publisher(SimTopic.coord_done(1)))

        try:
            async with coordinator_subprocess(
                binary=coordinator_bin,
                args=[
                    "--connect",
                    zenoh_router,
                    "--topology",
                    str(world_yaml),
                    "--pcap-log",
                    str(pcap_path),
                    "--nodes",
                    "2",
                ],
                zenoh_session=zenoh_session,
                liveliness_topic=SimTopic.COORD_ALIVE,
                env={"RUST_LOG": "debug"},
            ):
                q = 1
                u64_max = 0xFFFFFFFFFFFFFFFF
                while len(rx_received) < 3 and q < 10:
                    if q == 1:
                        done0 = vproto.CoordDoneReq(quantum=q, vtime_limit=u64_max, messages=[msg_eth, msg_uart1])
                        done1 = vproto.CoordDoneReq(quantum=q, vtime_limit=u64_max, messages=[msg_uart2])
                    else:
                        done0 = vproto.CoordDoneReq(quantum=q, vtime_limit=u64_max, messages=[])
                        done1 = vproto.CoordDoneReq(quantum=q, vtime_limit=u64_max, messages=[])

                    await asyncio.to_thread(pub0.put, done0.pack())
                    await asyncio.to_thread(pub1.put, done1.pack())

                    try:
                        await asyncio.wait_for(rx_event.wait(), timeout=0.2)
                    except TimeoutError:
                        q += 1

                # Wait for the next quantum start just to be sure coordinator fully flushed
                try:
                    await asyncio.wait_for(quantum_event.wait(), timeout=1.0)
                except TimeoutError:
                    pass
        finally:
            await asyncio.to_thread(sub.undeclare)
            await asyncio.to_thread(sub_eth.undeclare)
            await asyncio.to_thread(sub_uart.undeclare)
            await asyncio.to_thread(pub0.undeclare)
            await asyncio.to_thread(pub1.undeclare)

    pcap1 = tmp_path / "run1.pcap"
    await run_simulation(pcap1)

    pcap2 = tmp_path / "run2.pcap"
    await run_simulation(pcap2)

    assert pcap1.exists()
    assert pcap2.exists()

    with pcap1.open("rb") as f1, pcap2.open("rb") as f2:
        content1 = f1.read()
        content2 = f2.read()

    assert content1 == content2, "PCAP files are not bit-identical!"
    assert len(content1) > 24, "PCAP file only contains the global header, no packets written!"
