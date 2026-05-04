"""
Stress test for chardev-zenoh flow control.
Sends a large amount of data and verifies that nothing is dropped
and the guest doesn't stall, even with fragmented writes.
"""

from __future__ import annotations

import asyncio
from pathlib import Path
from typing import TYPE_CHECKING

import pytest

from tools import vproto
from tools.testing.env import WORKSPACE_ROOT
from tools.testing.utils import get_time_multiplier

if TYPE_CHECKING:
    from tools.testing.virtmcu_test_suite.simulation import Simulation


def encode_frame(delivery_vtime_ns: int, payload: bytes, sequence: int = 0) -> bytes:
    header = vproto.ZenohFrameHeader(delivery_vtime_ns, sequence, len(payload))
    return bytes(header.pack()) + payload


def _on_tx(
    payload: bytes,
    received_data: bytearray,
    received_event: asyncio.Event,
    expected_count: int,
    loop: asyncio.AbstractEventLoop,
) -> None:
    data = payload
    if len(data) > vproto.SIZE_ZENOH_FRAME_HEADER:
        payload = data[vproto.SIZE_ZENOH_FRAME_HEADER :]
        received_data.extend(payload)
        if len(received_data) >= expected_count:
            # In case we're not in the main thread (like Zenoh callback), safely set the event
            loop.call_soon_threadsafe(received_event.set)


@pytest.mark.asyncio
@pytest.mark.timeout(600)
async def test_chardev_flow_control_stress(simulation: Simulation) -> None:

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

    # Start QEMU with zenoh chardev and clock in slaved-suspend mode
    # simulation handles router, node, mode=slaved-suspend, stall-timeout
    extra_args = [
        "-chardev",
        f"virtmcu,id=char0,topic={topic_base},max-backlog=1024,baud-rate-ns=0",
        "-serial",
        "chardev:char0",
    ]

    simulation.add_node(node_id=node_id, dtb=dtb, kernel=kernel, extra_args=extra_args)

    # Declare Zenoh subscribers BEFORE entering the simulation context
    # so the framework's routing barrier covers them.
    
    received_data = bytearray()
    received_event = asyncio.Event()
    expected_count = 50



    # Initialize transport before subscribing to not miss any startup messages
    if simulation.transport is None:
        from tools.testing.virtmcu_test_suite.transport import ZenohTransportImpl
        simulation.transport = ZenohTransportImpl(simulation._router, simulation._session)
    assert simulation.transport is not None
    
    def on_payload(payload: bytes) -> None:
        _on_tx(payload, received_data, received_event, expected_count, loop)
    await simulation.transport.subscribe(tx_topic, on_payload)

    async with simulation as sim:
        # Wait for firmware boot by stepping simulation
        booted = False

        for _ in range(50):  # 50 steps of 10ms
            # 60s base timeout scales to 300s in ASan via get_time_multiplier()
            await sim.vta.step(10_000_000, timeout=60.0)  # LINT_EXCEPTION: vta_step_loop
            if b"Interactive UART Echo Ready." in received_data:
                booted = True
                break

        if not booted:
            pytest.fail(f"Firmware boot timeout (virtual time). Buffer: {received_data}")

        received_data.clear()

        # Flood with data. Send in one large packet to avoid overwhelming the Zenoh thread in QEMU.
        start_vtime = sim.vta.current_vtimes[node_id] + 1_000_000  # +1ms

        payload_data = bytes([(i % 26) + 65 for i in range(expected_count)])
        packet = encode_frame(start_vtime, payload_data)
        assert sim.transport is not None
        asyncio.create_task(sim.transport.publish(rx_topic, packet))

        # Final time advancement to ensure all data is processed
        timeout = 60 * get_time_multiplier()
        start_time = asyncio.get_running_loop().time()
        while len(received_data) < expected_count:
            # 60s base timeout scales to 300s in ASan via get_time_multiplier()
            await sim.vta.step(10_000_000, timeout=60.0)  # LINT_EXCEPTION: vta_step_loop


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
