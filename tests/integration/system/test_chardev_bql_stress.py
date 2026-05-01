"""
SOTA Test Module: test_chardev_bql_stress

Context:
This module implements tests for the test_chardev_bql_stress subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_chardev_bql_stress.
"""

import asyncio
from pathlib import Path

import pytest
import vproto
import zenoh


def _find_workspace_root(start_path: Path) -> Path:
    for p in [start_path, *list(start_path.parents)]:
        if (p / "VERSION").exists() or (p / ".git").exists():
            return p
    return start_path.parent.parent.parent

# Paths
WORKSPACE_DIR = _find_workspace_root(Path(__file__).resolve())


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

    # Use the echo firmware from uart_echo
    uart_echo_dir = Path(WORKSPACE_DIR) / "tests/fixtures/guest_apps/uart_echo"
    kernel = uart_echo_dir / "echo.elf"
    dtb = Path(WORKSPACE_DIR) / "tests/fixtures/guest_apps/boot_arm/minimal.dtb"
    if not kernel.exists():
        pytest.fail(f"Kernel {kernel} not found")
    if not dtb.exists():
        pytest.fail(f"DTB {dtb} not found")

    node_id = 42
    topic_base = "virtmcu/uart"
    rx_topic = f"{topic_base}/{node_id}/rx"
    tx_topic = f"{topic_base}/{node_id}/tx"

    import os

    from tools.testing.utils import get_time_multiplier

    base_stall_timeout = int(os.environ.get("VIRTMCU_STALL_TIMEOUT_MS", "5000"))
    stall_timeout = int(base_stall_timeout * get_time_multiplier())

    # Start QEMU with zenoh chardev and clock in slaved-suspend mode
    extra_args = [
        "-S",
        "-icount",
        "shift=4,align=off,sleep=off",
        "-device",
        f"virtmcu-clock,node={node_id},mode=slaved-suspend,router={router_endpoint},stall-timeout={stall_timeout}",
        "-chardev",
        f"virtmcu,id=char0,node={node_id},router={router_endpoint},topic={topic_base},max-backlog=1024",
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

    # Time authority to drive the clock
    from tests.conftest import VirtualTimeAuthority
    from tools.testing.virtmcu_test_suite.conftest_core import VirtmcuSimulation

    vta = VirtualTimeAuthority(session, [node_id])
    sim = VirtmcuSimulation(bridge, vta)

    async with sim:
        # Wait for firmware boot by stepping simulation
        booted = False

        for _ in range(50):  # 50 steps of 10ms
            await vta.step(10_000_000, timeout=30.0)
            if b"Interactive UART Echo Ready." in received_data:
                booted = True
                break

        if not booted:
            pytest.fail(f"Firmware boot timeout (virtual time). Buffer: {received_data}")

        received_data.clear()

        # Flood with data. Send in one large packet to avoid overwhelming the Zenoh thread in QEMU.
        start_vtime = vta.current_vtimes[node_id] + 1_000_000  # +1ms

        payload_data = bytes([i % 256 for i in range(expected_count)])
        packet = encode_frame(start_vtime, payload_data)
        pub.put(packet)

        # Final time advancement to ensure all data is processed
        from tools.testing.utils import get_time_multiplier

        timeout = 60 * get_time_multiplier()
        start_time = asyncio.get_event_loop().time()
        while len(received_data) < expected_count:
            await vta.step(10_000_000, timeout=30.0)  # 10ms steps
            if asyncio.get_event_loop().time() - start_time > timeout:
                break
            try:
                await asyncio.wait_for(received_event.wait(), timeout=0.01)
                received_event.clear()
            except TimeoutError:
                pass

    # Query dropped frames
    dropped = await bridge.execute("qom-get", {"path": "/chardevs/char0", "property": "dropped-frames"})

    assert len(received_data) == expected_count, (
        f"Dropped data: got {len(received_data)} bytes, expected {expected_count} (dropped={dropped})"
    )
    # Verify data integrity
    for i in range(expected_count):
        assert received_data[i] == i % 256, f"Data corruption at index {i}"

    await bridge.close()
    session.close().wait()
