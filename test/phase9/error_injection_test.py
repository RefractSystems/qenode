import os
import subprocess
import time

from mmio_client import MMIOClient

ADAPTER_PATH = "./tools/systemc_adapter/build/adapter"
SOCKET_PATH = "/tmp/error_test.sock"

def run_adapter(node_id=""):
    cmd = [ADAPTER_PATH, SOCKET_PATH]
    if node_id:
        cmd.append(node_id)
    return subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)

def connect_to_adapter(path, timeout=5):
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

def test_invalid_mmio_size():
    print("--- Testing Invalid MMIO Sizes ---")
    adapter = run_adapter()
    client = connect_to_adapter(SOCKET_PATH)
    if not client:
        print("Failed to connect")
        adapter.terminate()
        return

    try:
        # struct mmio_req { uint8_t type; uint8_t size; ... }
        # Let's try size 8 (not supported by adapter)
        # Note: mmio_client.write and read use fixed size 4 usually,
        # but let's force it.

        # Test read size 8
        val = client.read(0, size=8)
        print(f"Read size 8 returned: {val} (Expected 0 as adapter should return error)")

        # Test write size 1
        client.write(0, 0xAA, size=1)
        val = client.read(0, size=4)
        print(f"Read after write size 1: {hex(val)}")

        # Test write size 2
        client.write(4, 0xBBBB, size=2)
        val = client.read(4, size=4)
        print(f"Read after write size 2: {hex(val)}")

    finally:
        client.close()
        adapter.terminate()
        adapter.wait()

def test_abrupt_disconnect():
    print("--- Testing Abrupt Disconnect ---")
    adapter = run_adapter()
    client = connect_to_adapter(SOCKET_PATH)
    if not client:
        print("Failed to connect")
        adapter.terminate()
        return

    try:
        # Send a read but don't wait for response?
        # Actually MMIOClient.read waits.
        # Let's just close the socket while it's connected.
        client.close()
        print("Client closed socket.")
        time.sleep(1)

        # Reconnect should work because of my fix
        print("Attempting to reconnect...")
        client2 = connect_to_adapter(SOCKET_PATH)
        if client2:
            print("Reconnected successfully.")
            client2.write(0, 0x1234)
            val = client2.read(0)
            print(f"Read after reconnect: {hex(val)}")
            client2.close()
        else:
            print("Failed to reconnect!")

    finally:
        adapter.terminate()
        adapter.wait()

if __name__ == "__main__":
    test_invalid_mmio_size()
    test_abrupt_disconnect()
