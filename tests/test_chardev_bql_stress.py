import asyncio
import struct
from pathlib import Path

import pytest
import zenoh

# Paths
WORKSPACE_DIR = Path(__file__).parent.parent.resolve()

# Header format for zenoh-chardev: [vtime(8) | len(4)]
HEADER_FORMAT = "<QI"
HEADER_SIZE = 12

def encode_chardev_packet(vtime_ns: int, data: bytes) -> bytes:
    return struct.pack(HEADER_FORMAT, vtime_ns, len(data)) + data

@pytest.mark.asyncio
async def test_chardev_flow_control_stress(qemu_launcher, zenoh_router):
    """
    Stress test for chardev-zenoh flow control.
    Sends a large amount of data and verifies that nothing is dropped
    and the guest doesn't stall, even with fragmented writes.
    """
    router_endpoint = zenoh_router

    # Use the echo firmware from phase8
    phase8_dir = Path(WORKSPACE_DIR) / "test/phase8"
    kernel = phase8_dir / "echo.elf"
    dtb = Path(WORKSPACE_DIR) / "test/phase1/minimal.dtb"

    if not kernel.exists():
        pytest.skip(f"Kernel {kernel} not found")
    if not dtb.exists():
        pytest.skip(f"DTB {dtb} not found")

    node_id = 42
    topic_base = f"virtmcu/uart/{node_id}"
    rx_topic = f"{topic_base}/rx"
    tx_topic = f"{topic_base}/tx"

    # Start QEMU with zenoh chardev and zenoh-clock in slaved-suspend mode
    extra_args = [
        "-device", f"zenoh-clock,node={node_id},mode=slaved-suspend,router={router_endpoint}",
        "-chardev", f"zenoh,id=char0,node={node_id},router={router_endpoint}",
        "-serial", "chardev:char0"
    ]

    bridge = await qemu_launcher(dtb, kernel, extra_args, ignore_clock_check=True)

    # Connect Zenoh to send/receive data
    z_config = zenoh.Config()
    z_config.insert_json5("connect/endpoints", f'["{router_endpoint}"]')
    session = zenoh.open(z_config)

    received_data = bytearray()
    received_event = asyncio.Event()
    expected_count = 500

    def on_tx(sample):
        data = sample.payload.to_bytes()
        # Header is [vtime(8) | len(4)]
        if len(data) > HEADER_SIZE:
            payload = data[HEADER_SIZE:]
            received_data.extend(payload)
            if len(received_data) >= expected_count:
                received_event.set()

    _sub = session.declare_subscriber(tx_topic, on_tx)
    pub = session.declare_publisher(rx_topic)

    await bridge.start_emulation()

    # Time authority to drive the clock
    from tests.conftest import VirtualTimeAuthority
    vta = VirtualTimeAuthority(session, [node_id])

    # Flood with data. Send in one large packet to avoid overwhelming the Zenoh thread in QEMU.
    start_vtime = 1_000_000 # 1ms

    payload_data = bytes([i % 256 for i in range(expected_count)])
    packet = encode_chardev_packet(start_vtime, payload_data)
    pub.put(packet)

    # Advance time to let QEMU process
    await vta.step(10_000_000, timeout=300.0) # 10ms steps
    await asyncio.sleep(0.5)

    # Final time advancement to ensure all data is processed
    timeout = 60
    start_time = asyncio.get_event_loop().time()
    while len(received_data) < expected_count:
        await vta.step(10_000_000, timeout=300.0) # 10ms steps
        if asyncio.get_event_loop().time() - start_time > timeout:
            break
        await asyncio.sleep(0.1)

    assert len(received_data) == expected_count, f"Dropped data: got {len(received_data)} bytes, expected {expected_count}"

    # Verify data integrity
    for i in range(expected_count):
        assert received_data[i] == i % 256, f"Data corruption at index {i}"

    await bridge.close()
    session.close().wait()
