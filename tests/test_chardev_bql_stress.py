import os
import struct
import subprocess
import threading
import time

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

TOPIC_BASE = "sim/chardev"  # must match zenoh-chardev subscription
NODE_ID = "0"
PORT = 7449

# 10k packets at 1 µs spacing → timers fire rapidly, stressing BQL from timer callbacks
FLOOD_COUNT = 10_000
FLOOD_VTIME_START_NS = 10_000_000  # 10 ms — avoids spending instructions before first byte
FLOOD_VTIME_STEP_NS = 1_000  # 1 µs between bytes


@pytest.fixture
def zenoh_router():
    router_path = os.path.join(os.getcwd(), "tests/zenoh_router_persistent.py")
    proc = subprocess.Popen(
        ["python3", router_path, f"tcp/127.0.0.1:{PORT}"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    time.sleep(2)
    yield f"tcp/127.0.0.1:{PORT}"
    proc.terminate()
    proc.wait()


@pytest.fixture
def qemu_instance(zenoh_router):
    dtb = os.path.join(os.getcwd(), "test/phase1/minimal.dtb")
    kernel = os.path.join(os.getcwd(), "test/phase8/echo.elf")
    qmp_sock = "/tmp/qmp_bql_stress.sock"
    if os.path.exists(qmp_sock):
        os.remove(qmp_sock)

    # Standalone icount: QEMU runs freely without clock-sync blocking the main
    # loop, so QMP remains responsive throughout the flood.
    cmd = [
        "./scripts/run.sh",
        "--dtb",
        dtb,
        "-kernel",
        kernel,
        "-icount",
        "shift=6,align=off,sleep=off",
        "-chardev",
        f"zenoh,id=uart0,node=0,router={zenoh_router}",
        "-serial",
        "chardev:uart0",
        "-qmp",
        f"unix:{qmp_sock},server,nowait",
        "-display",
        "none",
        "-monitor",
        "none",
    ]

    proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(3)  # allow QEMU + Zenoh discovery to settle
    yield qmp_sock
    proc.terminate()
    proc.wait()


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
        if i % 500 == 0:
            time.sleep(0.005)  # throttle to avoid overwhelming Zenoh router

    session.close()


def test_qmp_responsiveness_under_flood(zenoh_router, qemu_instance):
    """
    Flood the chardev RX channel while polling QMP.  Asserts that:
    - avg QMP latency stays below 200 ms
    - max QMP latency stays below 1 s
    These thresholds are intentionally generous: the test catches deadlocks,
    not micro-latency regressions.
    """
    import asyncio

    from qemu.qmp import QMPClient

    flood_thread = threading.Thread(target=_flood_uart, args=(zenoh_router,), daemon=True)
    flood_thread.start()

    async def _poll() -> list[float]:
        client = QMPClient("bql-stress-tester")
        await client.connect(qemu_instance)
        latencies: list[float] = []
        for _ in range(20):
            t0 = time.monotonic()
            await client.execute("query-status")
            latencies.append(time.monotonic() - t0)
            await asyncio.sleep(0.1)
        await client.disconnect()
        return latencies

    latencies = asyncio.run(_poll())
    flood_thread.join(timeout=30)

    avg = sum(latencies) / len(latencies)
    mx = max(latencies)
    print(f"\nQMP latency under flood: avg={avg:.3f}s  max={mx:.3f}s")

    assert avg < 0.2, f"Average QMP latency too high under UART flood: {avg:.3f}s"
    assert mx < 1.0, f"Max QMP latency too high under UART flood: {mx:.3f}s"
