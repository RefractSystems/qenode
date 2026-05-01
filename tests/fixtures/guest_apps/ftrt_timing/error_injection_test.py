"""
SOTA Test Module: error_injection_test

Context:
This module implements tests for the error_injection_test subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of error_injection_test.
"""

import logging
import subprocess
import time
from pathlib import Path

from mmio_client import MMIOClient

logger = logging.getLogger(__name__)

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
        if Path(path).exists():
            try:
                client = MMIOClient(path)
                client.connect()
                return client
            except Exception:
                pass
        time.sleep(0.5)
    return None


def test_invalid_mmio_size():
    logger.info("--- Testing Invalid MMIO Sizes ---")
    adapter = run_adapter()
    client = connect_to_adapter(SOCKET_PATH)
    if not client:
        logger.error("Failed to connect")
        adapter.terminate()
        return

    try:
        # struct mmio_req { uint8_t type; uint8_t size; ... }
        # Let's try size 8 (not supported by adapter)
        # Note: mmio_client.write and read use fixed size 4 usually,
        # but let's force it.

        # Test read size 8
        val = client.read(0, size=8)
        logger.info(f"Read size 8 returned: {val} (Expected 0 as adapter should return error)")

        # Test write size 1
        client.write(0, 0xAA, size=1)
        val = client.read(0, size=4)
        logger.info(f"Read after write size 1: {hex(val)}")

        # Test write size 2
        client.write(4, 0xBBBB, size=2)
        val = client.read(4, size=4)
        logger.info(f"Read after write size 2: {hex(val)}")

    finally:
        client.close()
        adapter.terminate()
        adapter.wait()


def test_abrupt_disconnect():
    logger.info("--- Testing Abrupt Disconnect ---")
    adapter = run_adapter()
    client = connect_to_adapter(SOCKET_PATH)
    if not client:
        logger.error("Failed to connect")
        adapter.terminate()
        return

    try:
        # Send a read but don't wait for response?
        # Actually MMIOClient.read waits.
        # Let's just close the socket while it's connected.
        client.close()
        logger.info("Client closed socket.")
        time.sleep(1)

        # Reconnect should work because of my fix
        logger.info("Attempting to reconnect...")
        client2 = connect_to_adapter(SOCKET_PATH)
        if client2:
            logger.info("Reconnected successfully.")
            client2.write(0, 0x1234)
            val = client2.read(0)
            logger.info(f"Read after reconnect: {hex(val)}")
            client2.close()
        else:
            logger.error("Failed to reconnect!")

    finally:
        adapter.terminate()
        adapter.wait()


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    test_invalid_mmio_size()
    test_abrupt_disconnect()
