import os
import struct
import subprocess
import threading
import time

import zenoh
from mmio_client import MMIOClient

ADAPTER_PATH = "./tools/systemc_adapter/build/adapter"
SOCKET_PATH = "/tmp/stress_test.sock"

def run_adapter(test_name, node_id=""):
    cmd = [ADAPTER_PATH, SOCKET_PATH]
    if node_id:
        cmd.append(node_id)
    out = open(f"/tmp/adapter_{test_name}_stdout.log", "w")
    err = open(f"/tmp/adapter_{test_name}_stderr.log", "w")
    return subprocess.Popen(cmd, stdout=out, stderr=err), out, err

def wait_for_socket(path, timeout=5):
    start = time.time()
    while time.time() - start < timeout:
        if os.path.exists(path):
            return True
        time.sleep(0.1)
    return False

def test_rapid_mmio():
    print("--- Testing Rapid MMIO ---")
    adapter, out, err = run_adapter("mmio")
    if not wait_for_socket(SOCKET_PATH):
        print("Adapter failed to create socket")
        adapter.terminate()
        return

    try:
        client = MMIOClient(SOCKET_PATH)
        client.connect()

        start_time = time.time()
        count = 100
        for i in range(count):
            if i % 10 == 0: print(f"  MMIO {i}/{count}...")
            client.write(i % 256 * 4, i, vtime_ns=i*100)
            val = client.read(i % 256 * 4, vtime_ns=i*100 + 50)
            if val != i:
                print(f"Mismatch at {i}: {val} != {i}")
                break

        end_time = time.time()
        print(f"Finished {count} MMIO R/W cycles in {end_time - start_time:.2f}s")
        client.close()
    finally:
        adapter.terminate()
        adapter.wait()
        out.close()
        err.close()

def test_rapid_can():
    print("--- Testing Rapid CAN ---")
    adapter, out, err = run_adapter("can", "stress-node")
    if not wait_for_socket(SOCKET_PATH):
        print("Adapter failed to create socket")
        adapter.terminate()
        return False

    try:
        client = MMIOClient(SOCKET_PATH)
        client.connect()

        z_session = zenoh.open(zenoh.Config())
        z_pub = z_session.declare_publisher("sim/systemc/frame/stress-node/rx")

        count = 100
        received_count = 0

        def injector():
            for i in range(count):
                # CanWireFrame: vtime(8), size(4), id(4), data(4)
                # Inject with some delay between frames
                payload = struct.pack("<QIII", (i+1)*1000000, 8, 0x100 + i, 0x1000 + i)
                z_pub.put(payload)
                time.sleep(0.01)

        t = threading.Thread(target=injector)
        t.start()

        start_time = time.time()
        timeout = 10
        while received_count < count and time.time() - start_time < timeout:
            # Poll status register (0x0C) for bit 0 (rx_pending)
            # Advance time slowly to allow Zenoh to deliver
            status = client.read(0x0C, vtime_ns=(received_count+1)*2000000)
            if status & 1:
                rx_id = client.read(0x10)
                rx_data = client.read(0x14)
                client.write(0x18, 1) # Clear IRQ
                received_count += 1
                if received_count % 10 == 0: print(f"  CAN {received_count}/{count}...")
            else:
                time.sleep(0.05)

        print(f"Received {received_count}/{count} CAN frames in {time.time() - start_time:.2f}s")

        t.join()
        z_session.close()
        client.close()
    finally:
        adapter.terminate()
        adapter.wait()
        out.close()
        err.close()

    if received_count != count:
        print("CAN Stress test FAILED (timeout or missed frames)")
        return False
    return True

def test_causality_regression():
    print("--- Testing Causality Regression ---")
    adapter, out, err = run_adapter("causality")
    if not wait_for_socket(SOCKET_PATH):
        print("Adapter failed to create socket")
        adapter.terminate()
        return

    try:
        client = MMIOClient(SOCKET_PATH)
        client.connect()

        client.write(0, 0x1234, vtime_ns=1000)
        print("Attempting write with regressed vtime...")
        client.write(4, 0x5678, vtime_ns=500)

        val1 = client.read(0, vtime_ns=1100)
        val2 = client.read(4, vtime_ns=1200)

        print(f"Vals: {hex(val1)}, {hex(val2)}")
        client.close()
    finally:
        adapter.terminate()
        adapter.wait()
        out.close()
        err.close()

if __name__ == "__main__":
    test_rapid_mmio()
    can_ok = test_rapid_can()
    test_causality_regression()

    if not can_ok:
        exit(1)
