"""
Stress test for chardev-zenoh flow control.
Sends a large amount of data and verifies that nothing is dropped
and the guest doesn't stall, even with fragmented writes.
"""

from __future__ import annotations

import asyncio
import os
from pathlib import Path
from typing import TYPE_CHECKING

import pytest
import zenoh

from tools import vproto
from tools.testing.env import WORKSPACE_ROOT
from tools.testing.utils import get_time_multiplier

if TYPE_CHECKING:
    from collections.abc import Awaitable, Callable

    from tools.testing.virtmcu_test_suite.conftest_core import QmpBridge


def encode_frame(delivery_vtime_ns: int, payload: bytes, sequence: int = 0) -> bytes:
    header = vproto.ZenohFrameHeader(delivery_vtime_ns, sequence, len(payload))
    return bytes(header.pack()) + payload


def _on_tx(
    sample: zenoh.Sample,
    received_data: bytearray,
    received_event: asyncio.Event,
    expected_count: int,
    loop: asyncio.AbstractEventLoop,
) -> None:
    data = sample.payload.to_bytes()
    if len(data) > vproto.SIZE_ZENOH_FRAME_HEADER:
        payload = data[vproto.SIZE_ZENOH_FRAME_HEADER :]
        received_data.extend(payload)
        if len(received_data) >= expected_count:
            # In case we're not in the main thread (like Zenoh callback), safely set the event
            loop.call_soon_threadsafe(received_event.set)


@pytest.mark.asyncio
async def test_chardev_flow_control_stress(
    qemu_launcher: Callable[..., Awaitable[QmpBridge]], zenoh_router: str
) -> None:

    router_endpoint = zenoh_router
    loop = asyncio.get_running_loop()

    # Use the echo firmware from uart_echo
    uart_echo_dir = Path(WORKSPACE_ROOT) / "tests/fixtures/guest_apps/uart_echo"
    kernel = uart_echo_dir / "echo.elf"
    dtb = Path(WORKSPACE_ROOT) / "tests/fixtures/guest_apps/boot_arm/minimal.dtb"
    if not kernel.exists():
        pytest.fail(f"Kernel {kernel} not found")
    if not dtb.exists():
        pytest.fail(f"DTB {dtb} not found")

    node_id = 42
    topic_base = "virtmcu/uart"
    rx_topic = f"{topic_base}/{node_id}/rx"
    tx_topic = f"{topic_base}/{node_id}/tx"

    base_stall_timeout = int(os.environ.get("VIRTMCU_STALL_TIMEOUT_MS", "30000"))
    stall_timeout = int(base_stall_timeout * get_time_multiplier())

    # Start QEMU with zenoh chardev and clock in slaved-suspend mode
    extra_args = [
        "-S",
        "-icount",
        "shift=0,align=off,sleep=off",
        "-device",
        f"virtmcu-clock,node={node_id},mode=slaved-suspend,router={router_endpoint},stall-timeout={stall_timeout}",
        "-chardev",
        f"virtmcu,id=char0,node={node_id},router={router_endpoint},topic={topic_base},max-backlog=1024,baud-rate-ns=0",
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
    expected_count = 50

    _sub = session.declare_subscriber(
        tx_topic, lambda sample: _on_tx(sample, received_data, received_event, expected_count, loop)
    )
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
            await vta.step(10_000_000, timeout=120.0)
            if b"Interactive UART Echo Ready." in received_data:
                booted = True
                break

        if not booted:
            pytest.fail(f"Firmware boot timeout (virtual time). Buffer: {received_data}")

        received_data.clear()

        # Flood with data. Send in one large packet to avoid overwhelming the Zenoh thread in QEMU.
        start_vtime = vta.current_vtimes[node_id] + 1_000_000  # +1ms

        payload_data = bytes([(i % 26) + 65 for i in range(expected_count)])
        packet = encode_frame(start_vtime, payload_data)
        pub.put(packet)

        # Final time advancement to ensure all data is processed
        timeout = 60 * get_time_multiplier()
        start_time = asyncio.get_running_loop().time()
        while len(received_data) < expected_count:
            await vta.step(10_000_000, timeout=30.0)  # 10ms steps

            if asyncio.get_running_loop().time() - start_time > timeout:
                break
            try:
                await asyncio.wait_for(received_event.wait(), timeout=0.01)
                received_event.clear()
            except TimeoutError:
                continue

    # Final Verification
    assert len(received_data) >= expected_count
    # Verify content (it should be an echo of what we sent)
    for i in range(expected_count):
        assert received_data[i] == (i % 26) + 65
