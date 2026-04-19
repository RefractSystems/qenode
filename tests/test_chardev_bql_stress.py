import asyncio
import os
import struct
import subprocess
import time

import pytest
import zenoh
from qemu.qmp import QMPClient

# This test verifies that flooding the Zenoh UART does not deadlock the BQL
# or significantly degrade QMP responsiveness.

TOPIC_BASE = "virtmcu/uart"
NODE_ID = "0"
PORT = 7449

@pytest.fixture
def zenoh_router():
    router_path = os.path.join(os.getcwd(), "tests/zenoh_router_persistent.py")
    proc = subprocess.Popen(["python3", router_path, f"tcp/127.0.0.1:{PORT}"])
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

    cmd = [
        "./scripts/run.sh",
        "--dtb", dtb,
        "-kernel", kernel,
        "-icount", "shift=6,align=off,sleep=off",
        "-device", f"zenoh-clock,node=0,mode=slaved-icount,router={zenoh_router},stall-timeout=60000",
        "-chardev", f"zenoh,id=uart0,node=0,router={zenoh_router}",
        "-serial", "chardev:uart0",
        "-qmp", f"unix:{qmp_sock},server,nowait",
        "-display", "none", "-monitor", "none"
    ]

    proc = subprocess.Popen(cmd)
    time.sleep(2)
    yield qmp_sock
    proc.terminate()
    proc.wait()

async def qmp_poll(qmp_sock):
    client = QMPClient('bql-stress-tester')
    await client.connect(qmp_sock)
    latencies = []
    for _ in range(50):
        start = time.time()
        await client.execute('query-status')
        latencies.append(time.time() - start)
        await asyncio.sleep(0.1)
    await client.disconnect()
    return latencies

def flood_uart(router):
    conf = zenoh.Config()
    conf.insert_json5("mode", '"client"')
    conf.insert_json5("connect/endpoints", f'["{router}"]')
    session = zenoh.open(conf)
    pub = session.declare_publisher(f"{TOPIC_BASE}/{NODE_ID}/rx")

    # Send 10,000 packets of 1 byte each to trigger many BQL locks/unlocks
    for i in range(10000):
        vtime = 10_000_000 + (i * 1000)
        header = struct.pack("<QI", vtime, 1)
        pub.put(header + b"A")
        if i % 100 == 0:
            time.sleep(0.01)

    session.close()

@pytest.mark.asyncio
async def test_qmp_responsiveness_under_flood(zenoh_router, qemu_instance):
    import threading

    # Start flooding in a background thread
    flood_thread = threading.Thread(target=flood_uart, args=(zenoh_router,))
    flood_thread.start()

    # Start a Time Authority to advance clock so QEMU processes the RX
    def time_authority():
        conf = zenoh.Config()
        conf.insert_json5("mode", '"client"')
        conf.insert_json5("connect/endpoints", f'["{zenoh_router}"]')
        session = zenoh.open(conf)
        for _ in range(100):
            session.get("sim/clock/advance/0", payload=struct.pack("<QQ", 1_000_000, 0))
            time.sleep(0.01)
        session.close()

    ta_thread = threading.Thread(target=time_authority)
    ta_thread.start()

    # Measure QMP latency
    latencies = await qmp_poll(qemu_instance)

    flood_thread.join()
    ta_thread.join()

    avg_latency = sum(latencies) / len(latencies)
    max_latency = max(latencies)
    print(f"\nQMP Latency under UART flood: avg={avg_latency:.4f}s, max={max_latency:.4f}s")

    # Assert QMP remains responsive (average < 100ms, max < 500ms)
    assert avg_latency < 0.1, f"Average QMP latency too high: {avg_latency}"
    assert max_latency < 0.5, f"Max QMP latency too high: {max_latency}"
