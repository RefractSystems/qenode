import subprocess
import time
import os
import socket
import struct
import threading
import zenoh
from mmio_client import MMIOClient

ADAPTER_PATH = "./tools/systemc_adapter/build/adapter"
SOCKET_PATH = "/tmp/stress_test.sock"

VIRTMCU_PROTO_MAGIC = 0x564D4355
VIRTMCU_PROTO_VERSION = 1

def run_adapter(test_name, node_id=""):
    cmd = [ADAPTER_PATH, SOCKET_PATH]
    if node_id:
        cmd.append(node_id)
    out = open(f"/tmp/adapter_{test_name}_stdout.log", "w")
    err = open(f"/tmp/adapter_{test_name}_stderr.log", "w")
    return subprocess.Popen(cmd, stdout=out, stderr=err), out, err

def connect_to_adapter(path, timeout=10):
    start = time.time()
    while time.time() - start < timeout:
        if os.path.exists(path):
            try:
                client = MMIOClient(path)
                client.connect()
                return client
            except:
                pass
        time.sleep(0.5)
    return None

def test_rapid_mmio():
    print("--- Testing Rapid MMIO ---")
    adapter, out, err = run_adapter("mmio")
    client = connect_to_adapter(SOCKET_PATH)
    if not client:
        print("Adapter failed to create socket or handshake failed")
        adapter.terminate()
        return

    try:
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
    client = connect_to_adapter(SOCKET_PATH)
    if not client:
        print("Adapter failed to create socket")
        adapter.terminate()
        return False
    
    try:
        z_session = zenoh.open(zenoh.Config())
        z_pub = z_session.declare_publisher("sim/systemc/frame/stress-node/rx")
        
        count = 100
        received_count = 0
        
        def injector():
            for i in range(count):
                payload = struct.pack("<QIII", (i+1)*1000000, 8, 0x100 + i, 0x1000 + i)
                z_pub.put(payload)
                time.sleep(0.05) # Slower injection

        t = threading.Thread(target=injector)
        t.start()
        start_time = time.time()
        timeout = 30
        current_vtime = 1000000
        last_found_time = time.time()

        while received_count < count and time.time() - start_time < timeout:
            # Poll status at current_vtime
            status = client.read(0x0C, vtime_ns=current_vtime)
            # Drain FIFO
            while status & 1 and received_count < count:
                rx_id = client.read(0x10, vtime_ns=current_vtime)
                rx_data = client.read(0x14, vtime_ns=current_vtime)
                client.write(0x18, 1, vtime_ns=current_vtime) 
                received_count += 1
                last_found_time = time.time()
                if received_count % 10 == 0: print(f"  CAN RX {received_count}/{count} at vtime={current_vtime}...")
                status = client.read(0x0C, vtime_ns=current_vtime)
            
            if received_count < count:
                # If we haven't found a frame for a bit, advance vtime
                if time.time() - last_found_time > 0.1:
                     current_vtime += 1000000
                     last_found_time = time.time()
                time.sleep(0.02)

                
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
        print("CAN Stress test FAILED")
        return False
    return True

def test_can_tx():
    print("--- Testing CAN TX ---")
    adapter, out, err = run_adapter("can_tx", "tx-node")
    client = connect_to_adapter(SOCKET_PATH)
    if not client:
        print("Adapter failed to create socket")
        adapter.terminate()
        return False
    
    try:
        z_session = zenoh.open(zenoh.Config())
        z_sub_data = []
        def on_frame(sample):
            z_sub_data.append(sample.payload)
            
        z_sub = z_session.declare_subscriber("sim/systemc/frame/tx-node/tx", on_frame)
        
        count = 10
        for i in range(count):
            can_id = 0x200 + i
            can_data = 0x2000 + i
            vtime = (i + 1) * 1000000
            client.write(0x00, can_id, vtime_ns=vtime)
            client.write(0x04, can_data, vtime_ns=vtime + 100)
            client.write(0x08, 1, vtime_ns=vtime + 200)
            time.sleep(0.1)
            
        print(f"Waiting for {count} TX frames via Zenoh...")
        start_time = time.time()
        while len(z_sub_data) < count and time.time() - start_time < 5:
            time.sleep(0.1)
            
        print(f"Received {len(z_sub_data)}/{count} TX frames")
        z_session.close()
        client.close()
    finally:
        adapter.terminate()
        adapter.wait()
        out.close()
        err.close()
        
    if len(z_sub_data) != count:
        print("CAN TX test FAILED")
        return False
    return True

def test_causality_regression():
    print("--- Testing Causality Regression ---")
    adapter, out, err = run_adapter("causality")
    client = connect_to_adapter(SOCKET_PATH)
    if not client:
        print("Adapter failed to create socket")
        adapter.terminate()
        return

    try:
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
    tx_ok = test_can_tx()
    test_causality_regression()
    
    if not (can_ok and tx_ok):
        exit(1)
