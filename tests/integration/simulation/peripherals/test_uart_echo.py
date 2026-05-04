# ZENOH_HACK_EXCEPTION: Global exemption for now while tests are refactored.
"""
SOTA Test Module: test_uart_echo

Context:
This module implements tests for the test_uart_echo subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_uart_echo.
"""

from __future__ import annotations

import asyncio
import hashlib
import os
from typing import TYPE_CHECKING, Any

import pytest

from tools import vproto
from tools.testing.virtmcu_test_suite.topics import SimTopic

if TYPE_CHECKING:
    from pathlib import Path

    import zenoh

    from tools.testing.virtmcu_test_suite.simulation import Simulation


class ZenohUartMonitor:
    """Monitors UART traffic over Zenoh."""

    def __init__(self, session: zenoh.Session, node_id: int | str, base_topic: str, is_rx: bool = False) -> None:
        self.node_id = node_id
        self.is_rx = is_rx
        if is_rx:
            self.topic = f"{base_topic}/{node_id}/rx"
        else:
            self.topic = f"{base_topic}/{node_id}/tx"
        self.rx_topic = f"{base_topic}/{node_id}/rx"
        self.buffer = ""
        self.session = session
        self.sub: Any | None = None
        self.queue: asyncio.Queue[str] = asyncio.Queue()

    async def start(self) -> None:
        """Starts the Zenoh subscriber."""
        loop = asyncio.get_running_loop()
        self.queue = asyncio.Queue()

        def on_sample(sample: zenoh.Sample) -> None:
            payload = sample.payload.to_bytes()
            if len(payload) > 20:
                text = payload[vproto.SIZE_ZENOH_FRAME_HEADER :].decode("utf-8", errors="replace")
                loop.call_soon_threadsafe(self.queue.put_nowait, text)

        self.sub = await asyncio.to_thread(lambda: self.session.declare_subscriber(self.topic, on_sample))

    async def wait_for(self, pattern: str, timeout: float = 10.0) -> bool:
        """Waits for a pattern to appear in the monitor buffer."""
        start_time = asyncio.get_running_loop().time()
        while asyncio.get_running_loop().time() - start_time < timeout:
            try:
                chunk = await asyncio.wait_for(self.queue.get(), timeout=0.1)
                self.buffer += chunk
                if pattern in self.buffer:
                    return True
            except TimeoutError:
                pass
        return False

    async def stop(self) -> None:
        """Stops the Zenoh subscriber."""
        if self.sub:
            await asyncio.to_thread(self.sub.undeclare)


@pytest.mark.asyncio
async def test_interactive_echo(simulation: Simulation, tmp_path: Path) -> None:
    """
    Interactive UART Echo test (Unix Sockets).
    """
    from tools.testing.env import WORKSPACE_ROOT

    workspace_root = WORKSPACE_ROOT
    kernel = workspace_root / "tests/fixtures/guest_apps/uart_echo/echo.elf"
    dtb = tmp_path / "minimal.dtb"
    import shutil

    shutil.copy(workspace_root / "tests/fixtures/guest_apps/boot_arm/minimal.dtb", dtb)

    extra_args = ["-icount", "shift=4,align=off,sleep=off"]
    simulation.add_node(node_id=0, dtb=dtb, kernel=kernel, extra_args=extra_args)
    async with simulation as sim:
        await sim.node(0).wait_for_uart("Interactive UART Echo Ready.", timeout_ns=500_000_000)
        await sim.node(0).bridge.write_to_uart("Hello virtmcu\r")
        await sim.node(0).wait_for_uart("Hello virtmcu", timeout_ns=500_000_000)


@pytest.mark.asyncio
@pytest.mark.parametrize("deterministic_coordinator", [{"nodes": 2, "pdes": True}], indirect=True)
@pytest.mark.usefixtures("deterministic_coordinator")
async def test_multi_node_uart(simulation: Simulation, tmp_path: Path) -> None:
    """
    Multi-node UART over Zenoh.
    """
    from tools.testing.env import WORKSPACE_ROOT

    workspace_root = WORKSPACE_ROOT
    kernel = workspace_root / "tests/fixtures/guest_apps/uart_echo/echo.elf"
    dtb = tmp_path / "minimal.dtb"
    import shutil

    shutil.copy(workspace_root / "tests/fixtures/guest_apps/boot_arm/minimal.dtb", dtb)

    unique_id = hashlib.sha256(tmp_path.name.encode()).hexdigest()[:8]
    topic0 = SimTopic.uart_unique_prefix(f"n0_{unique_id}")
    topic1 = SimTopic.uart_unique_prefix(f"n1_{unique_id}")

    extra0 = [
        "-icount",
        "shift=4,align=off,sleep=off",
        "-device",
        "virtmcu-clock,coordinated=true",
        "-chardev",
        f"virtmcu,id=chr0,topic={topic0}",
        "-serial",
        "chardev:chr0",
    ]
    simulation.add_node(node_id=0, dtb=dtb, kernel=kernel, extra_args=extra0)

    extra1 = [
        "-icount",
        "shift=4,align=off,sleep=off",
        "-device",
        "virtmcu-clock,coordinated=true",
        "-chardev",
        f"virtmcu,id=chr0,topic={topic1}",
        "-serial",
        "chardev:chr0",
    ]
    simulation.add_node(node_id=1, dtb=dtb, kernel=kernel, extra_args=extra1)

    zenoh_session = simulation._session
    monitor0 = ZenohUartMonitor(zenoh_session, 0, topic0)
    monitor1_tx = ZenohUartMonitor(zenoh_session, 1, topic1)
    monitor1_rx = ZenohUartMonitor(zenoh_session, 1, topic1, is_rx=True)

    await monitor0.start()
    await monitor1_tx.start()
    await monitor1_rx.start()

    # Manual bridge: Node 0 TX -> Node 1 RX
    def bridge_cb(sample: zenoh.Sample) -> None:
        payload = sample.payload.to_bytes()
        zenoh_session.put(f"{topic1}/1/rx", payload)

    sub_bridge = await asyncio.to_thread(lambda: zenoh_session.declare_subscriber(f"{topic0}/0/tx", bridge_cb))

    async with simulation as sim:
        vta = sim.vta
        assert vta is not None

        # Helper to pump monitor queues
        def _pump_monitor(mon: ZenohUartMonitor) -> None:
            while not mon.queue.empty():
                mon.buffer += mon.queue.get_nowait()

        # 1. Boot both
        iters = 50 if os.environ.get("VIRTMCU_USE_ASAN") == "1" else 500
        for _ in range(iters):
            await vta.step(10_000_000)
            _pump_monitor(monitor0)
            _pump_monitor(monitor1_tx)
            _pump_monitor(monitor1_rx)
            if (
                "Interactive UART Echo Ready." in monitor0.buffer
                and "Interactive UART Echo Ready." in monitor1_tx.buffer
            ):
                break
        else:
            pytest.fail(f"Nodes boot timeout | mon0: {monitor0.buffer!r} | mon1_tx: {monitor1_tx.buffer!r}")

        # Clear buffers before starting the actual test phase to avoid false positives from welcome messages
        monitor0.buffer = ""
        monitor1_tx.buffer = ""
        monitor1_rx.buffer = ""

        # 2. Inject PING to Node 0 one by one for maximum reliability
        test_data = b"PING"
        for char in test_data:
            msg_byte = bytes([char])
            char_str = msg_byte.decode()

            # Ensure we are at a clean state
            await vta.step(5_000_000)
            vtime = vta.current_vtimes[0]
            header_bytes = vproto.ZenohFrameHeader(vtime + 1_000_000, 0, len(msg_byte)).pack()

            def _do_put(h: bytes = header_bytes, m: bytes = msg_byte) -> None:
                zenoh_session.put(monitor0.rx_topic, h + m)

            await asyncio.to_thread(_do_put)

            # Wait for echo on both Node 0 (direct) and Node 1 (bridged)
            for _ in range(100):  # Increased retry count
                await vta.step(5_000_000)
                _pump_monitor(monitor0)
                _pump_monitor(monitor1_rx)
                if char_str in monitor0.buffer and char_str in monitor1_rx.buffer:
                    break
            else:
                pytest.fail(
                    f"Echo failed for {char_str!r} | mon0: {monitor0.buffer!r} | mon1_rx: {monitor1_rx.buffer!r}"
                )

            # Remove the character from buffers to prepare for next one
            monitor0.buffer = monitor0.buffer.replace(char_str, "", 1)
            monitor1_rx.buffer = monitor1_rx.buffer.replace(char_str, "", 1)

    await monitor0.stop()
    await monitor1_tx.stop()
    await monitor1_rx.stop()
    await asyncio.to_thread(sub_bridge.undeclare)


@pytest.mark.asyncio
@pytest.mark.parametrize("deterministic_coordinator", [{"nodes": 1, "pdes": True}], indirect=True)
@pytest.mark.usefixtures("deterministic_coordinator")
async def test_coordinator_topology(simulation: Simulation, tmp_path: Path) -> None:
    """
    Test Zenoh coordinator topology control (Packet Drop).
    """
    from tools.testing.env import WORKSPACE_ROOT

    workspace_root = WORKSPACE_ROOT
    kernel = workspace_root / "tests/fixtures/guest_apps/uart_echo/echo.elf"
    dtb = tmp_path / "minimal.dtb"
    import shutil

    shutil.copy(workspace_root / "tests/fixtures/guest_apps/boot_arm/minimal.dtb", dtb)

    unique_id = hashlib.sha256(tmp_path.name.encode()).hexdigest()[:8]
    topic = SimTopic.uart_unique_prefix(f"top_{unique_id}")

    extra = [
        "-icount",
        "shift=4,align=off,sleep=off",
        "-device",
        "virtmcu-clock,coordinated=true",
        "-chardev",
        f"virtmcu,id=chr0,topic={topic}",
        "-serial",
        "chardev:chr0",
    ]
    simulation.add_node(node_id=0, dtb=dtb, kernel=kernel, extra_args=extra)

    zenoh_session = simulation._session
    monitor = ZenohUartMonitor(zenoh_session, 0, topic)
    await monitor.start()

    # Register dummy node 1
    dummy_topic = f"{topic}/1/tx"

    def _reg() -> None:
        zenoh_session.put(dummy_topic, vproto.ZenohFrameHeader(0, 0, 1).pack() + b"H")

    await asyncio.to_thread(_reg)

    async with simulation as sim:
        vta = sim.vta
        assert vta is not None

        def _pump_monitor(mon: ZenohUartMonitor) -> None:
            while not mon.queue.empty():
                mon.buffer += mon.queue.get_nowait()

        # 1. Start and wait for welcome
        iters = 50 if os.environ.get("VIRTMCU_USE_ASAN") == "1" else 500
        for _ in range(iters):
            await vta.step(10_000_000)
            _pump_monitor(monitor)
            if "Interactive UART Echo Ready." in monitor.buffer:
                break
        else:
            pytest.fail(f"Node 0 boot timeout | mon: {monitor.buffer!r}")

        # Clear buffer to ensure subsequent checks are clean
        monitor.buffer = ""

        # 3. Apply topology: Drop all from Node 0 to Node 1
        import json

        ctrl_topic = SimTopic.NETWORK_CONTROL
        update = {"from": "0", "to": "1", "drop_probability": 1.0}
        await asyncio.to_thread(lambda: zenoh_session.put(ctrl_topic, json.dumps(update)))
        # We step the clock slightly to ensure the coordinator process receives the update
        await vta.step(1_000_000)

        # 4. Monitor Node 1 RX
        received_msgs: list[bytes] = []

        def on_node1_rx(sample: zenoh.Sample) -> None:
            payload = sample.payload.to_bytes()
            if len(payload) > 20:
                received_msgs.append(payload[vproto.SIZE_ZENOH_FRAME_HEADER :])

        sub1_rx = await asyncio.to_thread(lambda: zenoh_session.declare_subscriber(f"{topic}/1/rx", on_node1_rx))

        # 5. Inject P1 (X) to Node 0, it should NOT reach Node 1 RX
        msg = b"X"
        header = vproto.ZenohFrameHeader(vta.current_vtimes[0] + 1_000_000, 0, len(msg)).pack()
        await asyncio.to_thread(lambda: zenoh_session.put(monitor.rx_topic, header + msg))

        for _ in range(50):
            await vta.step(10_000_000)
            _pump_monitor(monitor)
            if "X" in monitor.buffer:
                break
        else:
            pytest.fail("Node 0 did not echo X")

        assert not any(b"X" in m for m in received_msgs), "Message X was NOT dropped by coordinator"

        # 6. Enable and verify with P2 (Y)
        update = {"from": "0", "to": "1", "drop_probability": 0.0}
        await asyncio.to_thread(lambda: zenoh_session.put(ctrl_topic, json.dumps(update)))
        await vta.step(1_000_000)

        msg = b"Y"
        header = vproto.ZenohFrameHeader(vta.current_vtimes[0] + 1_000_000, 0, len(msg)).pack()
        await asyncio.to_thread(lambda: zenoh_session.put(monitor.rx_topic, header + msg))

        for _ in range(50):
            await vta.step(10_000_000)
            _pump_monitor(monitor)
            if "Y" in monitor.buffer:
                break
        else:
            pytest.fail("Node 0 did not echo Y")

        # Extra step to ensure routing through coordinator finishes
        await vta.step(1_000_000)

        assert any(b"Y" in m for m in received_msgs), "Marker packet Y did not arrive"
        await monitor.stop()
        await asyncio.to_thread(sub1_rx.undeclare)


@pytest.mark.asyncio
async def test_uart_stress(simulation: Simulation, tmp_path: Path) -> None:
    """
    UART Stress test using slaved-icount and large bursts.
    """
    from tools.testing.env import WORKSPACE_ROOT

    workspace_root = WORKSPACE_ROOT
    kernel = workspace_root / "tests/fixtures/guest_apps/uart_echo/echo.elf"
    dtb = tmp_path / "minimal.dtb"
    import shutil

    shutil.copy(workspace_root / "tests/fixtures/guest_apps/boot_arm/minimal.dtb", dtb)

    unique_id = hashlib.sha256(tmp_path.name.encode()).hexdigest()[:8]
    topic = SimTopic.uart_unique_prefix(f"stress_{unique_id}")

    extra = [
        "-icount",
        "shift=4,align=off,sleep=off",
        "-device",
        "virtmcu-clock,coordinated=false",
        "-chardev",
        f"virtmcu,id=uart0,topic={topic}",
        "-serial",
        "chardev:uart0",
    ]
    simulation.add_node(node_id=0, dtb=dtb, kernel=kernel, extra_args=extra)

    zenoh_session = simulation._session
    monitor = ZenohUartMonitor(zenoh_session, 0, topic)
    await monitor.start()

    async with simulation as sim:
        vta = sim.vta
        assert vta is not None

        def _pump_monitor(mon: ZenohUartMonitor) -> None:
            while not mon.queue.empty():
                mon.buffer += mon.queue.get_nowait()

        iters = 50 if os.environ.get("VIRTMCU_USE_ASAN") == "1" else 500
        for _ in range(iters):
            await vta.step(10_000_000)
            _pump_monitor(monitor)
            if "Interactive UART Echo Ready." in monitor.buffer:
                # Clear buffer before burst test
                monitor.buffer = ""
                break
        else:
            pytest.fail("Stress test boot timeout")

        # Blast a 128-byte burst (Exceeds PL011 32-byte FIFO)
        burst_data = b"BURST_TEST_" * 12 + b"END"  # ~135 bytes
        header = vproto.ZenohFrameHeader(vta.current_vtimes[0] + 1_000_000, 0, len(burst_data)).pack()

        def _do_burst() -> None:
            zenoh_session.put(f"{topic}/0/rx", header + burst_data)

        await asyncio.to_thread(_do_burst)

        # Advance and verify full echo
        iters = 50 if os.environ.get("VIRTMCU_USE_ASAN") == "1" else 500
        for _ in range(iters):
            await vta.step(10_000_000)
            _pump_monitor(monitor)
            if burst_data.decode() in monitor.buffer:
                break
        else:
            pytest.fail(f"Burst data not fully echoed. Received buffer length: {len(monitor.buffer)}")

    await monitor.stop()
