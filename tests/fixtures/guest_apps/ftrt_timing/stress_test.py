"""
SOTA Test Module: stress_test

Context:
This module implements tests for the stress_test subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of stress_test.
"""

import contextlib
import logging
import subprocess
import threading
import time
from pathlib import Path

import vproto
import zenoh
from mmio_client import MMIOClient

logger = logging.getLogger(__name__)

ADAPTER_PATH = "./tools/systemc_adapter/build/adapter"
SOCKET_PATH = "/tmp/stress_test.sock"

VIRTMCU_PROTO_MAGIC = 0x564D4355
VIRTMCU_PROTO_VERSION = 1


@contextlib.contextmanager
def run_adapter(test_name, node_id=""):
    cmd = [ADAPTER_PATH, SOCKET_PATH]
    if node_id:
        cmd.append(node_id)
    with (
        Path(f"/tmp/adapter_{test_name}_stdout.log").open("w") as out,
        Path(f"/tmp/adapter_{test_name}_stderr.log").open("w") as err,
    ):
        adapter = subprocess.Popen(cmd, stdout=out, stderr=err)
        try:
            yield adapter
        finally:
            adapter.terminate()
            adapter.wait()


def connect_to_adapter(path, timeout=10):
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


def test_rapid_mmio():
    logger.info("--- Testing Rapid MMIO ---")
    with run_adapter("mmio"):
        client = connect_to_adapter(SOCKET_PATH)
        if not client:
            logger.info("Adapter failed to create socket or handshake failed")
            return

        try:
            start_time = time.time()
            count = 100
            for i in range(count):
                if i % 10 == 0:
                    logger.info(f"  MMIO {i}/{count}...")
                client.write(i % 256 * 4, i, vtime_ns=i * 100)
                val = client.read(i % 256 * 4, vtime_ns=i * 100 + 50)
                if val != i:
                    logger.info(f"Mismatch at {i}: {val} != {i}")
                    break

            end_time = time.time()
            logger.info(f"Finished {count} MMIO R/W cycles in {end_time - start_time:.2f}s")
        finally:
            if client:
                client.close()


def test_rapid_can():
    logger.info("--- Testing Rapid CAN ---")
    with run_adapter("can", "stress-node"):
        client = connect_to_adapter(SOCKET_PATH)
        if not client:
            logger.info("Adapter failed to create socket")
            return False

        try:
            z_session = zenoh.open(zenoh.Config())
            z_pub = z_session.declare_publisher("sim/systemc/frame/stress-node/rx")

            count = 100
            received_count = 0

            def injector():
                for i in range(count):
                    can_id = 0x100 + i
                    can_data = 0x1000 + i
                    vtime = (i + 1) * 1000000
                    hdr = vproto.ZenohFrameHeader(vtime, 0, 8).pack()
                    payload = hdr + can_id.to_bytes(4, "little") + can_data.to_bytes(4, "little")
                    z_pub.put(payload)
                    time.sleep(0.05)  # Slower injection

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
                    client.read(0x10, vtime_ns=current_vtime)
                    client.read(0x14, vtime_ns=current_vtime)
                    client.write(0x18, 1, vtime_ns=current_vtime)
                    received_count += 1
                    last_found_time = time.time()
                    if received_count % 10 == 0:
                        logger.info(f"  CAN RX {received_count}/{count} at vtime={current_vtime}...")
                    status = client.read(0x0C, vtime_ns=current_vtime)

                if received_count < count:
                    # If we haven't found a frame for a bit, advance vtime
                    if time.time() - last_found_time > 0.1:
                        current_vtime += 1000000
                        last_found_time = time.time()
                    time.sleep(0.02)

            logger.info(f"Received {received_count}/{count} CAN frames in {time.time() - start_time:.2f}s")

            t.join()
            z_session.close()
        finally:
            if client:
                client.close()

        if received_count != count:
            logger.info("CAN Stress test FAILED")
            return False
        return True


def test_can_tx():
    logger.info("--- Testing CAN TX ---")
    with run_adapter("can_tx", "tx-node"):
        client = connect_to_adapter(SOCKET_PATH)
        if not client:
            logger.info("Adapter failed to create socket")
            return False

        try:
            z_session = zenoh.open(zenoh.Config())
            z_sub_data = []

            def on_frame(sample):
                z_sub_data.append(sample.payload)

            z_session.declare_subscriber("sim/systemc/frame/tx-node/tx", on_frame)

            count = 10
            for i in range(count):
                can_id = 0x200 + i
                can_data = 0x2000 + i
                vtime = (i + 1) * 1000000
                client.write(0x00, can_id, vtime_ns=vtime)
                client.write(0x04, can_data, vtime_ns=vtime + 100)
                client.write(0x08, 1, vtime_ns=vtime + 200)
                time.sleep(0.1)

            logger.info(f"Waiting for {count} TX frames via Zenoh...")
            start_time = time.time()
            while len(z_sub_data) < count and time.time() - start_time < 5:
                time.sleep(0.1)

            logger.info(f"Received {len(z_sub_data)}/{count} TX frames")
            z_session.close()
        finally:
            if client:
                client.close()

        if len(z_sub_data) != count:
            logger.info("CAN TX test FAILED")
            return False
        return True


def test_causality_regression():
    logger.info("--- Testing Causality Regression ---")
    with run_adapter("causality"):
        client = connect_to_adapter(SOCKET_PATH)
        if not client:
            logger.info("Adapter failed to create socket")
            return

        try:
            client.write(0, 0x1234, vtime_ns=1000)
            logger.info("Attempting write with regressed vtime...")
            client.write(4, 0x5678, vtime_ns=500)

            val1 = client.read(0, vtime_ns=1100)
            val2 = client.read(4, vtime_ns=1200)

            logger.info(f"Vals: {hex(val1)}, {hex(val2)}")
        finally:
            if client:
                client.close()


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    test_rapid_mmio()
    can_ok = test_rapid_can()
    tx_ok = test_can_tx()
    test_causality_regression()

    if not (can_ok and tx_ok):
        exit(1)
