import asyncio
import struct
import threading
import time
from pathlib import Path

import pytest
import zenoh

# Verify that flooding the Zenoh UART chardev with RX packets does not deadlock
# the BQL or degrade QMP responsiveness.
#
# Clock sync (slaved-icount) is intentionally absent: when QEMU blocks at each
# quantum boundary the QMP socket is unavailable during that period, which would
# make the latency assertions meaningless.  This test validates the BQL/QMP
# path under *standalone* icount — the scenario where UART traffic is the only
# source of BQL contention.

TOPIC_BASE = "virtmcu/uart"  # must match zenoh-chardev subscription
NODE_ID = "0"

# 10k packets at 1 µs spacing → timers fire rapidly, stressing BQL from timer callbacks
# We use a slightly smaller payload/spacing to ensure QEMU can keep up without OOMing the socket buffer
FLOOD_COUNT = 5_000
FLOOD_VTIME_START_NS = 10_000_000  # 10 ms — avoids spending instructions before first byte
FLOOD_VTIME_STEP_NS = 1_000  # 1 µs between bytes


def _flood_uart(router: str) -> None:
    conf = zenoh.Config()
    conf.insert_json5("mode", '"client"')
    conf.insert_json5("connect/endpoints", f'["{router}"]')
    session = zenoh.open(conf)
    pub = session.declare_publisher(f"{TOPIC_BASE}/{NODE_ID}/rx")

    for i in range(FLOOD_COUNT):
        vtime = FLOOD_VTIME_START_NS + (i * FLOOD_VTIME_STEP_NS)
        header = struct.pack("<QI", vtime, 1)
        pub.put(header + b"A")
        if i % 100 == 0:
            time.sleep(0.01)  # throttle to avoid overwhelming Zenoh router

    session.close()


@pytest.mark.asyncio
async def test_qmp_responsiveness_under_flood(zenoh_router, qemu_launcher):
    """
    Flood the chardev RX channel while polling QMP.  Asserts that:
    - avg QMP latency stays below 200 ms
    - max QMP latency stays below 1 s
    These thresholds are intentionally generous: the test catches deadlocks,
    not micro-latency regressions.
    """
    dtb = Path(Path.cwd()) / "test/phase1/minimal.dtb"
    kernel = Path(Path.cwd()) / "test/phase8/echo.elf"

    # Standalone icount: QEMU runs freely without clock-sync blocking the main
    # loop, so QMP remains responsive throughout the flood.
    extra_args = [
        "-icount", "shift=6,align=off,sleep=off",
        "-chardev", f"zenoh,id=uart0,node=0,router={zenoh_router}",
        "-serial", "chardev:uart0",
        "-S",
    ]

    # ignore_clock_check=True bypasses the wait for sim/clock/advance
    bridge = await qemu_launcher(dtb, kernel, extra_args=extra_args, ignore_clock_check=True)

    # We use a Zenoh subscriber to wait for the firmware's "Interactive UART Echo Ready" message
    # This provides a fast and deterministic signal that QEMU has booted and is ready to be flooded.
    conf = zenoh.Config()
    conf.insert_json5("mode", '"client"')
    conf.insert_json5("connect/endpoints", f'["{zenoh_router}"]')
    session = zenoh.open(conf)

    ready_event = asyncio.Event()
    loop = asyncio.get_running_loop()
    rx_buffer = bytearray()

    def rx_handler(sample: zenoh.Sample):
        nonlocal rx_buffer
        payload = sample.payload.to_bytes()
        if len(payload) > 12:
            # 12 byte header (8 byte vtime, 4 byte res)
            rx_buffer.extend(payload[12:])
            text = rx_buffer.decode("utf-8", errors="ignore")
            if "Interactive UART Echo Ready." in text:
                loop.call_soon_threadsafe(ready_event.set)

    sub = session.declare_subscriber(f"{TOPIC_BASE}/{NODE_ID}/tx", rx_handler)

    await bridge.start_emulation()

    try:
        await asyncio.wait_for(ready_event.wait(), timeout=10.0)
    except TimeoutError:
        pytest.fail("Firmware did not output ready message over Zenoh chardev.")
    finally:
        sub.undeclare()
        session.close()

    # Wait a tiny bit for the firmware to settle into its wfi() loop
    await asyncio.sleep(0.5)

    flood_thread = threading.Thread(target=_flood_uart, args=(zenoh_router,), daemon=True)
    flood_thread.start()

    latencies: list[float] = []
    try:
        for _ in range(20):
            t0 = time.monotonic()
            await asyncio.wait_for(bridge.qmp.execute("query-status"), timeout=2.0)
            latencies.append(time.monotonic() - t0)
            await asyncio.sleep(0.1)
    except TimeoutError:
        pytest.fail("QMP query-status timed out! BQL is deadlocked.")
    except Exception as e:
        print(f"QMP ERROR: {e}")
        await asyncio.sleep(0.5)
        pytest.fail("QEMU crashed or QMP disconnected during flood!")

    await asyncio.to_thread(flood_thread.join, 30)

    avg = sum(latencies) / len(latencies)
    mx = max(latencies)
    print(f"\nQMP latency under flood: avg={avg:.3f}s  max={mx:.3f}s")

    assert avg < 0.2, f"Average QMP latency too high under UART flood: {avg:.3f}s"
    assert mx < 1.0, f"Max QMP latency too high under UART flood: {mx:.3f}s"
