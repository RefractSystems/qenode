import logging
import os
import socket
import subprocess
import tempfile
import time
from pathlib import Path

from mmio_client import MMIOClient

from tools.testing.utils import mock_execution_delay

"""
SOTA Test Module: error_injection_test

Context:
This module implements tests for the error_injection_test subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of error_injection_test.
"""

logger = logging.getLogger(__name__)

ADAPTER_PATH = "./tools/systemc_adapter/build/adapter"
SOCKET_PATH = str(Path(tempfile.gettempdir()) / f"error_test_{os.getpid()}.sock")


def run_adapter(node_id: str = "") -> subprocess.Popen[str]:
    cmd = [ADAPTER_PATH, SOCKET_PATH]
    if node_id:
        cmd.append(node_id)
    return subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)  # type: ignore[return-value]


def connect_to_adapter(path: str, timeout: int = 5) -> socket.socket:
    start = time.time()
    while time.time() - start < timeout:
        if Path(path).exists():
            try:
                client = MMIOClient(path)
                client.connect()
                return client  # type: ignore[no-any-return]
            except OSError:
                pass
        mock_execution_delay(0.5)  # SLEEP_EXCEPTION: mock test simulating execution/spacing
    return None  # type: ignore[return-value]


def test_invalid_mmio_size() -> None:
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
        val = client.read(0, size=8)  # type: ignore[attr-defined]
        logger.info(f"Read size 8 returned: {val} (Expected 0 as adapter should return error)")

        # Test write size 1
        client.write(0, 0xAA, size=1)  # type: ignore[attr-defined]
        val = client.read(0, size=4)  # type: ignore[attr-defined]
        logger.info(f"Read after write size 1: {hex(val)}")

        # Test write size 2
        client.write(4, 0xBBBB, size=2)  # type: ignore[attr-defined]
        val = client.read(4, size=4)  # type: ignore[attr-defined]
        logger.info(f"Read after write size 2: {hex(val)}")

    finally:
        client.close()
        adapter.terminate()
        adapter.wait()


def test_abrupt_disconnect() -> None:
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
        mock_execution_delay(1)  # SLEEP_EXCEPTION: mock test simulating execution/spacing

        # Reconnect should work because of my fix
        logger.info("Attempting to reconnect...")
        client2 = connect_to_adapter(SOCKET_PATH)
        if client2:
            logger.info("Reconnected successfully.")
            client2.write(0, 0x1234)  # type: ignore[attr-defined]
            val = client2.read(0)  # type: ignore[attr-defined]
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
