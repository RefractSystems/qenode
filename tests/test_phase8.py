import asyncio
import contextlib
import struct
import subprocess
from pathlib import Path

import pytest


@pytest.mark.xdist_group(name="serial-clock")
@pytest.mark.asyncio
async def test_phase8_interactive_echo(qemu_launcher):
    """
    Phase 8: Interactive UART Echo test.
    Migrated from tests/test_interactive_echo.robot
    """
    workspace_root = Path(Path(Path(__file__).parent.resolve().parent))
    dtb = Path(workspace_root) / "test/phase1/minimal.dtb"
    kernel = Path(workspace_root) / "test/phase8/echo.elf"

    if not Path(kernel).exists():
        subprocess.run(["make", "-C", "test/phase8"], check=True)

    bridge = await qemu_launcher(dtb, kernel, extra_args=["-S"])
    await bridge.start_emulation()

    # 1. Wait for welcome message
    assert await bridge.wait_for_line_on_uart("Interactive UART Echo Ready.")
    assert await bridge.wait_for_line_on_uart("Type something:")

    # 2. Type some characters
    await bridge.write_to_uart("Hello virtmcu\r")

    # 3. Verify they are echoed back
    assert await bridge.wait_for_line_on_uart("Hello virtmcu")


@pytest.mark.xdist_group(name="serial-clock")
@pytest.mark.asyncio
async def test_phase8_multi_node_uart(zenoh_router, zenoh_coordinator, qemu_launcher, zenoh_session):  # noqa: ARG001
    """
    Phase 8: Multi-node UART over Zenoh.
    Verify Node 1 sending UART data reaches Node 2 via Zenoh coordinator.
    """
    workspace_root = Path(Path(Path(__file__).parent.resolve().parent))
    dtb = Path(workspace_root) / "test/phase1/minimal.dtb"
    kernel = Path(workspace_root) / "test/phase8/echo.elf"

    import uuid

    topic = f"virtmcu/uart/{uuid.uuid4().hex[:8]}"

    # Start node1
    extra1 = [
        "-S",
        "-chardev",
        f"zenoh,id=chr0,node=0,router={zenoh_router},topic={topic}",
        "-serial",
        "chardev:chr0",
    ]
    bridge1 = await qemu_launcher(dtb, kernel, extra_args=extra1, ignore_clock_check=True)

    # Start node2
    extra2 = [
        "-S",
        "-chardev",
        f"zenoh,id=chr0,node=1,router={zenoh_router},topic={topic}",
        "-serial",
        "chardev:chr0",
    ]
    bridge2 = await qemu_launcher(dtb, kernel, extra_args=extra2, ignore_clock_check=True)

    # Helper to wait for string on Zenoh UART topic
    class ZenohUartMonitor:
        def __init__(self, session, node_id, base_topic):
            self.topic = f"{base_topic}/{node_id}/tx"
            self.rx_topic = f"{base_topic}/{node_id}/rx"
            self.queue: asyncio.Queue[str] = asyncio.Queue()
            self.buffer = ""
            self.session = session
            self.sub = None

        async def start(self):
            loop = asyncio.get_running_loop()

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

    monitor0 = ZenohUartMonitor(zenoh_session, 0, topic)
    monitor1 = ZenohUartMonitor(zenoh_session, 1, topic)
    await monitor0.start()
    await monitor1.start()

    await bridge1.start_emulation()
    await bridge2.start_emulation()

    # Wait for welcome on both via Zenoh
    assert await monitor0.wait_for("Interactive UART Echo Ready.")
    assert await monitor1.wait_for("Interactive UART Echo Ready.")

    # Node 1 should echo back its TX topic (which we monitor via monitor1)
    # Wait for discovery
    await asyncio.sleep(0.1)

    # Inject message into node0's RX topic
    msg = b"Message from Zenoh\r"
    header = struct.pack("<QI", 0, len(msg))
    await asyncio.to_thread(lambda: zenoh_session.put(monitor0.rx_topic, header + msg))

    # Node 0 should echo it back to its TX topic
    assert await monitor0.wait_for("Message from Zenoh")

    # And coordinator should have routed it to Node 1's RX topic
    assert await monitor1.wait_for("Message from Zenoh")


@pytest.mark.xdist_group(name="serial-clock")
@pytest.mark.asyncio
async def test_phase8_uart_hammer(zenoh_router, qemu_launcher, zenoh_session):
    """
    Phase 8: UART Hammer Test.
    Blasts 100,000 individual 1-byte Zenoh messages to stress the TX aggregation
    and non-blocking background threads.
    """
    workspace_root = Path(__file__).parent.parent
    dtb = Path(workspace_root) / "test/phase1/minimal.dtb"
    kernel = Path(workspace_root) / "test/phase8/echo.elf"

    if not Path(kernel).exists():
        subprocess.run(["make", "-C", "test/phase8"], check=True)

    import uuid

    topic = f"virtmcu/uart/{uuid.uuid4().hex[:8]}"

    extra_args = [
        "-icount",
        "shift=4,align=off,sleep=off",
        "-device",
        f"zenoh-clock,node=0,mode=slaved-icount,router={zenoh_router},stall-timeout=120000",
        "-chardev",
        f"zenoh,id=uart0,node=0,router={zenoh_router},topic={topic}",
        "-serial",
        "chardev:uart0",
    ]

    bridge = await qemu_launcher(dtb, kernel, extra_args=extra_args, ignore_clock_check=True)
    await bridge.start_emulation()

    # Wait for welcome
    await bridge.wait_for_line("Interactive UART Echo Ready.", timeout=10.0)

    pub = await asyncio.to_thread(lambda: zenoh_session.declare_publisher(f"{topic}/0/rx"))

    received_count = 0
    received_all_event = asyncio.Event()

    def on_tx_sample(sample):
        nonlocal received_count
        payload = sample.payload.to_bytes()
        if len(payload) > 12:
            data = payload[12:]
            received_count += data.count(b"H")
            if received_count >= 100_000:
                received_all_event.set()

    sub = await asyncio.to_thread(lambda: zenoh_session.declare_subscriber(f"{topic}/0/tx", on_tx_sample))

    # Hammer it: 100,000 bytes in chunks
    chunk_size = 100
    for _i in range(100_000 // (chunk_size * 10)):
        tasks = []
        for _ in range(10):
            header = struct.pack("<QI", 0, chunk_size)
            data = b"H" * chunk_size

            def do_put(h=header, d=data):
                return pub.put(h + d)

            tasks.append(asyncio.to_thread(do_put))
        await asyncio.gather(*tasks)

        # Advance clock to let QEMU process
        await bridge.qmp.execute("sim-clock-advance", {"node-id": 0, "delta-ns": 10_000_000, "mujoco-active": False})

    try:
        await asyncio.wait_for(received_all_event.wait(), timeout=60.0)
    except TimeoutError:
        pytest.fail(f"Hammer test timed out - received {received_count}/100000 bytes")
    finally:
        await asyncio.to_thread(sub.undeclare)


@pytest.mark.xdist_group(name="serial-clock")
@pytest.mark.asyncio
async def test_phase8_uart_stress(zenoh_router, qemu_launcher, zenoh_session):
    """
    Phase 8: UART Stress Test.
    Sends 50,000 bytes at high speed and verifies they are all echoed back.
    """
    workspace_root = Path(Path(Path(__file__).parent.resolve().parent))
    dtb = Path(workspace_root) / "test/phase1/minimal.dtb"
    kernel = Path(workspace_root) / "test/phase8/echo.elf"

    if not Path(kernel).exists():
        subprocess.run(["make", "-C", "test/phase8"], check=True)

    import uuid

    topic = f"virtmcu/uart/{uuid.uuid4().hex[:8]}"

    extra_args = [
        "-icount",
        "shift=4,align=off,sleep=off",
        "-device",
        f"zenoh-clock,node=0,mode=slaved-icount,router={zenoh_router},stall-timeout=120000",
        "-chardev",
        f"zenoh,id=uart0,node=0,router={zenoh_router},topic={topic}",
        "-serial",
        "chardev:uart0",
    ]

    bridge = await qemu_launcher(dtb, kernel, extra_args=extra_args, ignore_clock_check=True)
    await bridge.start_emulation()

    TOTAL_BYTES = 50_000  # noqa: N806
    TEST_BYTE = b"X"  # noqa: N806
    TEST_BYTE_VAL = ord("X")  # noqa: N806
    BAUD_10MBPS_INTERVAL_NS = 2000  # noqa: N806
    START_VTIME_NS = 10_000_000  # noqa: N806
    QUANTUM_NS = 500_000_000  # noqa: N806
    CLOCK_TOTAL_NS = 20_000_000_000  # noqa: N806, 20s to be safe

    received_count = 0
    received_all_event = asyncio.Event()
    welcome_event = asyncio.Event()
    loop = asyncio.get_event_loop()

    def on_tx_sample(sample):
        raw = sample.payload.to_bytes()
        if b"Interactive UART Echo Ready." in raw:
            loop.call_soon_threadsafe(welcome_event.set)

        if len(raw) < 12:
            return
        payload = raw[12:]
        new_x = sum(1 for b in payload if b == TEST_BYTE_VAL)

        def _update():
            nonlocal received_count
            received_count += new_x
            if received_count >= TOTAL_BYTES:
                received_all_event.set()

        loop.call_soon_threadsafe(_update)

    sub = await asyncio.to_thread(lambda: zenoh_session.declare_subscriber(f"{topic}/0/tx", on_tx_sample))

    # Wait for discovery propagation
    await asyncio.sleep(2.0)

    # 1. Drive clock until welcome message (ensures Zenoh discovery is complete)
    welcome_wait_vtime = 0
    while not welcome_event.is_set() and welcome_wait_vtime < 10_000_000_000:
        replies = await asyncio.to_thread(
            lambda: list(
                zenoh_session.get("sim/clock/advance/0", payload=struct.pack("<QQ", QUANTUM_NS, 0), timeout=5.0)
            )
        )
        if not replies or not replies[0].ok:
            await asyncio.sleep(0.1)
            continue

        payload = replies[0].ok.payload.to_bytes()
        welcome_wait_vtime, _, _ = struct.unpack("<QII", payload)

    pub = await asyncio.to_thread(lambda: zenoh_session.declare_publisher(f"{topic}/0/rx"))
    await asyncio.sleep(0.5)

    # 2. Pre-publish all bytes
    def _publish_all():
        for i in range(TOTAL_BYTES):
            vtime = START_VTIME_NS + (i * BAUD_10MBPS_INTERVAL_NS)
            header = struct.pack("<QI", vtime, 1)
            pub.put(header + TEST_BYTE)
            if i % 1000 == 0:
                import time

                time.sleep(0.01)  # Pace publication to avoid overflowing Zenoh buffers

    await asyncio.to_thread(_publish_all)

    # Drive clock
    current_vtime = welcome_wait_vtime
    while current_vtime < CLOCK_TOTAL_NS and not received_all_event.is_set():
        # Step clock
        replies = await asyncio.to_thread(
            lambda: list(
                zenoh_session.get("sim/clock/advance/0", payload=struct.pack("<QQ", QUANTUM_NS, 0), timeout=5.0)
            )
        )
        if not replies or not replies[0].ok:
            await asyncio.sleep(0.1)
            continue

        payload = replies[0].ok.payload.to_bytes()
        current_vtime, _, _ = struct.unpack("<QII", payload)

    with contextlib.suppress(asyncio.TimeoutError):
        await asyncio.wait_for(received_all_event.wait(), timeout=30.0)
    assert received_count == TOTAL_BYTES
    await asyncio.to_thread(sub.undeclare)


@pytest.mark.xdist_group(name="serial-clock")
@pytest.mark.asyncio
async def test_phase8_uart_flood(zenoh_router, qemu_launcher, zenoh_session):
    """
    Phase 8: UART Flood Test.
    Blasts 50,000 bytes into the UART RX without proper headers or pacing.
    Verifies that QEMU remains stable (doesn't crash).
    """
    workspace_root = Path(Path(Path(__file__).parent.resolve().parent))
    dtb = Path(workspace_root) / "test/phase1/minimal.dtb"
    kernel = Path(workspace_root) / "test/phase8/echo.elf"

    extra_args = [
        "-icount",
        "shift=4,align=off,sleep=off",
        "-device",
        f"zenoh-clock,node=0,mode=slaved-icount,router={zenoh_router},stall-timeout=5000",
        "-chardev",
        f"zenoh,id=uart0,node=0,router={zenoh_router}",
        "-serial",
        "chardev:uart0",
    ]

    bridge = await qemu_launcher(dtb, kernel, extra_args=extra_args, ignore_clock_check=True)
    await bridge.start_emulation()

    pub = await asyncio.to_thread(lambda: zenoh_session.declare_publisher("virtmcu/uart/0/rx"))

    # Blast 50k bytes
    payload = b"X" * 50000
    await asyncio.to_thread(lambda: pub.put(payload))

    # Drive clock for a bit to see if it survives
    for _ in range(10):
        await asyncio.to_thread(
            lambda: list(
                zenoh_session.get("sim/clock/advance/0", payload=struct.pack("<QQ", 10_000_000, 0), timeout=1.0)
            )
        )

    # Check if QEMU is still alive (bridge.qmp.is_connected() or just try a command)
    await bridge.qmp.execute("query-status")
