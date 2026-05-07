"""
SOTA Test Module: test_wireshark_observability
Ensures the Wireshark extcap plugin and Zenoh PCAP dumper correctly capture
and format VirtMCU simulation traffic.
"""

import asyncio
from pathlib import Path

import pytest
import zenoh

from tools.testing.virtmcu_test_suite.conftest_core import ManagedSubprocess


@pytest.mark.asyncio
async def test_wireshark_extcap_capture(zenoh_router: str, zenoh_session: zenoh.Session, tmp_path: Path) -> None:
    """Verifies that virtmcu_extcap.py correctly captures Zenoh traffic into a PCAP file."""
    pcap_path = tmp_path / "test.pcap"

    # 1. Start the extcap plugin in capture mode
    extcap_cmd = [
        "python3",
        "tools/wireshark/virtmcu_extcap.py",
        "--capture",
        "--fifo",
        str(pcap_path),
        "--session",
        zenoh_router,
        "--topic",
        "sim/coord/**/rx",
    ]

    async with ManagedSubprocess("extcap", extcap_cmd) as _proc:
        # Wait a bit for it to connect and write the global header
        await asyncio.sleep(2)  # virtmcu-allow: sleep reasoning="observability capture wait"
        # 2. Publish some CoordMessages to Zenoh
        from tools import vproto

        vtime = 1_234_567_890
        src = 1
        dst = 2
        proto = 1  # Uart in main.rs serialize_protocol
        payload = b"HELLO WIRESHARK"

        msg = vproto.CoordMessage(
            src_node_id=src,
            dst_node_id=dst,
            delivery_vtime_ns=vtime,
            sequence_number=0,
            protocol=proto,
            payload=payload,
        )
        msg_data = msg.pack()

        zenoh_session.put("sim/coord/2/rx", msg_data)

        # Wait for capture
        await asyncio.sleep(2)  # virtmcu-allow: sleep reasoning="observability capture wait"
    # 3. Verify PCAP file
    assert pcap_path.exists()
    with pcap_path.open("rb") as f:
        data = f.read()

    # PCAP Global Header (24 bytes)
    assert len(data) >= 24
    assert data[:4] == b"\xd4\xc3\xb2\xa1"

    # Packet Header (16 bytes) + DLT_USER0 Header (10 bytes) + Payload
    packet_start = 24
    ts_sec = int.from_bytes(data[packet_start : packet_start + 4], "little")  # virtmcu-allow: int_from_bytes reasoning="Legacy exception"
    ts_usec = int.from_bytes(data[packet_start + 4 : packet_start + 8], "little")  # virtmcu-allow: int_from_bytes reasoning="Legacy exception"
    assert ts_sec == 1
    assert ts_usec == 234_567

    # DLT_USER0 Header
    dlt_start = packet_start + 16
    p_src = int.from_bytes(data[dlt_start : dlt_start + 4], "little")  # virtmcu-allow: int_from_bytes reasoning="Legacy exception"
    p_dst = int.from_bytes(data[dlt_start + 4 : dlt_start + 8], "little")  # virtmcu-allow: int_from_bytes reasoning="Legacy exception"
    p_proto = int.from_bytes(data[dlt_start + 8 : dlt_start + 10], "little")  # virtmcu-allow: int_from_bytes reasoning="Legacy exception"
    assert p_src == src
    assert p_dst == dst
    assert p_proto == 2  # UART
    assert data[dlt_start + 10 : dlt_start + 10 + len(payload)] == payload


@pytest.mark.asyncio
async def test_wireshark_extcap_stress(zenoh_router: str, zenoh_session: zenoh.Session, tmp_path: Path) -> None:
    """Stress tests the PCAP dumper with 1000 high-frequency messages."""
    pcap_path = tmp_path / "stress.pcap"
    num_messages = 1000

    extcap_cmd = [
        "python3",
        "tools/wireshark/virtmcu_extcap.py",
        "--capture",
        "--fifo",
        str(pcap_path),
        "--session",
        zenoh_router,
        "--topic",
        "sim/coord/**/rx",
    ]

    async with ManagedSubprocess("extcap_stress", extcap_cmd) as _proc:
        await asyncio.sleep(2)  # virtmcu-allow: sleep reasoning="observability capture wait"
        from tools import vproto

        payload = b"STRESS"
        for i in range(num_messages):
            vtime = (i + 1) * 1_000_000
            msg = vproto.CoordMessage(
                src_node_id=1,
                dst_node_id=2,
                delivery_vtime_ns=vtime,
                sequence_number=i,
                protocol=0,
                payload=payload,
            )
            msg_data = msg.pack()
            zenoh_session.put("sim/coord/2/rx", msg_data)

            if i % 100 == 0:
                await asyncio.sleep(0.01)  # virtmcu-allow: sleep reasoning="yield to let dumper process"
        await asyncio.sleep(3)  # virtmcu-allow: sleep reasoning="observability capture wait"
    assert pcap_path.exists()
    with pcap_path.open("rb") as f:
        data = f.read()

    expected_packet_size = 16 + 10 + 6
    num_packets = (len(data) - 24) // expected_packet_size
    assert num_packets >= num_messages


@pytest.mark.asyncio
async def test_wireshark_extcap_legacy_capture(zenoh_router: str, zenoh_session: zenoh.Session, tmp_path: Path) -> None:
    """Verifies that virtmcu_extcap.py correctly captures legacy ZenohFrameHeader traffic."""
    pcap_path = tmp_path / "test_legacy.pcap"

    extcap_cmd = [
        "python3",
        "tools/wireshark/virtmcu_extcap.py",
        "--capture",
        "--fifo",
        str(pcap_path),
        "--session",
        zenoh_router,
        "--topic",
        "sim/comm/**",
        "--legacy",
    ]

    async with ManagedSubprocess("extcap_legacy", extcap_cmd) as _proc:
        await asyncio.sleep(2)  # virtmcu-allow: sleep reasoning="observability capture wait"
        from tools import vproto

        vtime = 2_000_000_000
        payload = b"LEGACY TRAFFIC"
        header = vproto.ZenohFrameHeader(vtime, 0, len(payload)).pack()

        zenoh_session.put("sim/comm/uart/5/rx", header + payload)

        await asyncio.sleep(2)  # virtmcu-allow: sleep reasoning="observability capture wait"
    assert pcap_path.exists()
    with pcap_path.open("rb") as f:
        data = f.read()

    assert len(data) > 24 + 16 + 10
    packet_start = 24
    ts_sec = int.from_bytes(  # virtmcu-allow: int_from_bytes reasoning="Legacy exception"
        data[packet_start : packet_start + 4], "little"
    )
    assert ts_sec == 2
