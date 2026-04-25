import asyncio
import os
import struct
import uuid
from pathlib import Path

import pytest

from tests.conftest import wait_for_zenoh_discovery


class ZenohUartMonitor:
    def __init__(self, session, node_id, base_topic, is_rx=False):
        self.node_id = node_id
        self.is_rx = is_rx
        if is_rx:
            self.topic = f"{base_topic}/{node_id}/rx"
        else:
            self.topic = f"{base_topic}/{node_id}/tx"
        self.rx_topic = f"{base_topic}/{node_id}/rx"
        self.buffer = ""
        self.session = session
        self.sub = None

    async def start(self):
        loop = asyncio.get_running_loop()
        self.queue: asyncio.Queue[str] = asyncio.Queue()

        def on_sample(sample):
            payload = sample.payload.to_bytes()
            if len(payload) > 12:
                text = payload[12:].decode("utf-8", errors="replace")
                loop.call_soon_threadsafe(self.queue.put_nowait, text)

        self.sub = await asyncio.to_thread(lambda: self.session.declare_subscriber(self.topic, on_sample))

    async def wait_for(self, pattern, timeout=10.0):
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

    async def stop(self):
        if self.sub:
            await asyncio.to_thread(self.sub.undeclare)


@pytest.mark.asyncio
async def test_phase8_interactive_echo(qemu_launcher, tmp_path):
    """
    Phase 8: Interactive UART Echo test (Unix Sockets).
    """
    workspace_root = Path(__file__).parent.parent
    kernel = workspace_root / "test/phase8/echo.elf"
    dtb = tmp_path / "minimal.dtb"
    import shutil

    shutil.copy(workspace_root / "test/phase1/minimal.dtb", dtb)

    bridge = await qemu_launcher(dtb, kernel, extra_args=["-S"])
    await bridge.start_emulation()

    assert await bridge.wait_for_line_on_uart("Interactive UART Echo Ready.")
    await bridge.write_to_uart("Hello virtmcu\r")
    assert await bridge.wait_for_line_on_uart("Hello virtmcu")


@pytest.mark.asyncio
async def test_phase8_multi_node_uart(zenoh_router, zenoh_coordinator, qemu_launcher, zenoh_session, tmp_path):  # noqa: ARG001
    """
    Phase 8: Multi-node UART over Zenoh.
    Uses slaved-icount for absolute determinism.
    Uses echo.elf on Node 1 to avoid WFI stall and manual bridging to prevent hub storms.
    """
    workspace_root = Path(__file__).parent.parent
    kernel = workspace_root / "test/phase8/echo.elf"
    dtb = tmp_path / "minimal.dtb"
    import shutil

    shutil.copy(workspace_root / "test/phase1/minimal.dtb", dtb)

    topic0 = f"virtmcu/uart/n0_{uuid.uuid4().hex[:8]}"
    topic1 = f"virtmcu/uart/n1_{uuid.uuid4().hex[:8]}"

    extra0 = [
        "-S",
        "-icount",
        "shift=4,align=off,sleep=off",
        "-device",
        f"zenoh-clock,node=0,mode=slaved-icount,router={zenoh_router}",
        "-chardev",
        f"zenoh,id=chr0,node=0,router={zenoh_router},topic={topic0}",
        "-serial",
        "chardev:chr0",
    ]
    bridge0 = await qemu_launcher(dtb, kernel, extra_args=extra0, ignore_clock_check=True)

    extra1 = [
        "-S",
        "-icount",
        "shift=4,align=off,sleep=off",
        "-device",
        f"zenoh-clock,node=1,mode=slaved-icount,router={zenoh_router}",
        "-chardev",
        f"zenoh,id=chr0,node=1,router={zenoh_router},topic={topic1}",
        "-serial",
        "chardev:chr0",
    ]
    bridge1 = await qemu_launcher(dtb, kernel, extra_args=extra1, ignore_clock_check=True)

    monitor0 = ZenohUartMonitor(zenoh_session, 0, topic0)
    monitor1_tx = ZenohUartMonitor(zenoh_session, 1, topic1)
    monitor1_rx = ZenohUartMonitor(zenoh_session, 1, topic1, is_rx=True)

    await monitor0.start()
    await monitor1_tx.start()
    await monitor1_rx.start()

    # Manual bridge: Node 0 TX -> Node 1 RX
    def bridge_cb(sample):
        payload = sample.payload.to_bytes()
        zenoh_session.put(f"{topic1}/1/rx", payload)

    sub_bridge = await asyncio.to_thread(lambda: zenoh_session.declare_subscriber(f"{topic0}/0/tx", bridge_cb))

    await wait_for_zenoh_discovery(zenoh_session, f"{topic0}/0/tx")
    await bridge0.start_emulation()
    await bridge1.start_emulation()

    from tests.conftest import VirtualTimeAuthority

    vta = VirtualTimeAuthority(zenoh_session, [0, 1])

    # Helper to pump monitor queues
    def _pump_monitor(mon):
        while not mon.queue.empty():
            mon.buffer += mon.queue.get_nowait()

    # 1. Boot both
    iters = 50 if os.environ.get("VIRTMCU_USE_ASAN") == "1" else 500
    for _ in range(iters):
        await vta.step(10_000_000)
        _pump_monitor(monitor0)
        _pump_monitor(monitor1_tx)
        if "Interactive UART Echo Ready." in monitor0.buffer and "Interactive UART Echo Ready." in monitor1_tx.buffer:
            break
    else:
        pytest.fail("Nodes boot timeout")

    # 2. Inject PING to Node 0 one by one for maximum reliability
    test_data = b"PING"
    for char in test_data:
        msg_byte = bytes([char])

        await vta.step(5_000_000)
        vtime = vta.current_vtimes[0]
        header_bytes = struct.pack("<QI", vtime + 1_000_000, len(msg_byte))

        def _do_put(h=header_bytes, m=msg_byte):
            zenoh_session.put(monitor0.rx_topic, h + m)

        await asyncio.to_thread(_do_put)

        for _ in range(50):
            await vta.step(5_000_000)
            _pump_monitor(monitor0)
            _pump_monitor(monitor1_rx)
            if msg_byte.decode() in monitor0.buffer and msg_byte.decode() in monitor1_rx.buffer:
                break
        else:
            pytest.fail(f"Echo failed for {msg_byte.decode()}")

        monitor0.buffer = monitor0.buffer.replace(msg_byte.decode(), "", 1)
        monitor1_rx.buffer = monitor1_rx.buffer.replace(msg_byte.decode(), "", 1)

    await monitor0.stop()
    await monitor1_tx.stop()
    await monitor1_rx.stop()
    await asyncio.to_thread(sub_bridge.undeclare)


@pytest.mark.asyncio
async def test_phase8_coordinator_topology(zenoh_router, zenoh_coordinator, zenoh_session, qemu_launcher, tmp_path):  # noqa: ARG001
    """
    Phase 8: Test Zenoh coordinator topology control (Packet Drop).
    Uses the Marker Packet pattern to empirically prove a drop occurred.
    """
    workspace_root = Path(__file__).parent.parent
    kernel = workspace_root / "test/phase8/echo.elf"
    dtb = tmp_path / "minimal.dtb"
    import shutil

    shutil.copy(workspace_root / "test/phase1/minimal.dtb", dtb)

    topic = f"virtmcu/uart/top_{uuid.uuid4().hex[:8]}"

    extra = [
        "-S",
        "-icount",
        "shift=4,align=off,sleep=off",
        "-device",
        f"zenoh-clock,node=0,mode=slaved-icount,router={zenoh_router}",
        "-chardev",
        f"zenoh,id=chr0,node=0,router={zenoh_router},topic={topic}",
        "-serial",
        "chardev:chr0",
    ]
    bridge = await qemu_launcher(dtb, kernel, extra_args=extra, ignore_clock_check=True)

    monitor = ZenohUartMonitor(zenoh_session, 0, topic)
    await monitor.start()

    # Register dummy node 1
    dummy_topic = f"{topic}/1/tx"

    def _reg():
        zenoh_session.put(dummy_topic, struct.pack("<QI", 0, 1) + b"H")

    await asyncio.to_thread(_reg)

    await wait_for_zenoh_discovery(zenoh_session, f"{topic}/0/tx")
    await bridge.start_emulation()

    from tests.conftest import VirtualTimeAuthority

    vta = VirtualTimeAuthority(zenoh_session, [0])

    def _pump_monitor(mon):
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
        pytest.fail("Node 0 boot timeout")

    # 3. Apply topology: Drop all from Node 0 to Node 1
    import json

    ctrl_topic = "sim/network/control"
    update = {"from": "0", "to": "1", "drop_probability": 1.0}
    await asyncio.to_thread(lambda: zenoh_session.put(ctrl_topic, json.dumps(update)))
    # We step the clock slightly to ensure the coordinator process receives the update
    await vta.step(1_000_000)

    # 4. Monitor Node 1 RX
    received_msgs = []

    def on_node1_rx(sample):
        payload = sample.payload.to_bytes()
        if len(payload) > 12:
            received_msgs.append(payload[12:])

    sub1_rx = await asyncio.to_thread(lambda: zenoh_session.declare_subscriber(f"{topic}/1/rx", on_node1_rx))

    # 5. Inject P1 (X) to Node 0, it should NOT reach Node 1 RX
    msg = b"X"
    header = struct.pack("<QI", vta.current_vtimes[0] + 1_000_000, len(msg))
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
    header = struct.pack("<QI", vta.current_vtimes[0] + 1_000_000, len(msg))
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
async def test_phase8_uart_stress(zenoh_router, qemu_launcher, zenoh_session, tmp_path):
    """
    Phase 8: UART Stress test using slaved-icount and large bursts.
    Verifies that the zenoh-chardev flow control fix works.
    """
    workspace_root = Path(__file__).parent.parent
    kernel = workspace_root / "test/phase8/echo.elf"
    dtb = tmp_path / "minimal.dtb"
    import shutil

    shutil.copy(workspace_root / "test/phase1/minimal.dtb", dtb)

    topic = f"virtmcu/uart/stress_{uuid.uuid4().hex[:8]}"

    extra = [
        "-S",
        "-icount",
        "shift=4,align=off,sleep=off",
        "-device",
        f"zenoh-clock,node=0,mode=slaved-icount,router={zenoh_router}",
        "-chardev",
        f"zenoh,id=uart0,node=0,router={zenoh_router},topic={topic}",
        "-serial",
        "chardev:uart0",
    ]
    bridge = await qemu_launcher(dtb, kernel, extra_args=extra, ignore_clock_check=True)

    monitor = ZenohUartMonitor(zenoh_session, 0, topic)
    await monitor.start()

    await wait_for_zenoh_discovery(zenoh_session, f"{topic}/0/tx")
    await bridge.start_emulation()

    from tests.conftest import VirtualTimeAuthority

    vta = VirtualTimeAuthority(zenoh_session, [0])

    def _pump_monitor(mon):
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
    # This verifies the flow control and backlog implementation in zenoh-chardev
    burst_data = b"BURST_TEST_" * 12 + b"END"  # ~135 bytes
    header = struct.pack("<QI", vta.current_vtimes[0] + 1_000_000, len(burst_data))

    def _do_burst():
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
