import asyncio
from pathlib import Path

import pytest
import vproto
import zenoh

# Paths
WORKSPACE_DIR = Path(__file__).parent.parent.resolve()


def encode_frame(vtime_ns: int, data: bytes) -> bytes:
    # 24-byte ZenohFrameHeader (u64 delivery_vtime_ns, u64 sequence_number, u32 size + 4 padding)
    return vproto.ZenohFrameHeader(vtime_ns, 0, len(data)).pack() + data


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
        pytest.fail(f"Kernel {kernel} not found")
    if not dtb.exists():
        pytest.fail(f"DTB {dtb} not found")

    node_id = 42
    topic_base = f"virtmcu/uart/{node_id}"
    rx_topic = f"{topic_base}/rx"
    tx_topic = f"{topic_base}/tx"

    # Start QEMU with zenoh chardev and clock in slaved-suspend mode
    extra_args = [
        "-device",
        f"virtmcu-clock,node={node_id},mode=slaved-suspend,router={router_endpoint}",
        "-chardev",
        f"virtmcu,id=char0,node={node_id},router={router_endpoint}",
        "-serial",
        "chardev:char0",
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
        if len(data) > vproto.SIZE_ZENOH_FRAME_HEADER:
            payload = data[vproto.SIZE_ZENOH_FRAME_HEADER :]
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
    start_vtime = 1_000_000  # 1ms

    payload_data = bytes([i % 256 for i in range(expected_count)])
    packet = encode_frame(start_vtime, payload_data)
    pub.put(packet)

    # Final time advancement to ensure all data is processed
    from tools.testing.utils import get_time_multiplier

    timeout = 60 * get_time_multiplier()
    start_time = asyncio.get_event_loop().time()
    while len(received_data) < expected_count:
        await vta.step(10_000_000, timeout=300.0)  # 10ms steps
        if asyncio.get_event_loop().time() - start_time > timeout:
            break
        try:
            await asyncio.wait_for(received_event.wait(), timeout=0.01)
            received_event.clear()
        except TimeoutError:
            pass

    assert len(received_data) == expected_count, (
        f"Dropped data: got {len(received_data)} bytes, expected {expected_count}"
    )

    # Verify data integrity
    for i in range(expected_count):
        assert received_data[i] == i % 256, f"Data corruption at index {i}"

    await bridge.close()
    session.close().wait()
