"""
SOTA Test Module: bql_deadlock_test

Context:
This module implements tests for the bql_deadlock_test subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of bql_deadlock_test.
"""

import json
import logging
import socket
import sys
import threading
import time
import traceback
from pathlib import Path

import zenoh


def _find_workspace_root(start_path: Path) -> Path:
    for p in [start_path, *list(start_path.parents)]:
        if (p / "VERSION").exists() or (p / ".git").exists():
            return p
    return start_path.parent.parent.parent.parent

SCRIPT_DIR = Path(__file__).resolve().parent
WORKSPACE_DIR = _find_workspace_root(SCRIPT_DIR)
TOOLS_DIR = str(WORKSPACE_DIR / "tools")

if TOOLS_DIR not in sys.path:
    sys.path.insert(0, TOOLS_DIR)

from vproto import ClockAdvanceReq, ClockReadyResp  # noqa: E402

logger = logging.getLogger(__name__)

QMP_SOCK = sys.argv[1]
TOPIC = "sim/clock/advance/0"
TIMEOUT_S = 10.0
QMP_TIMEOUT_S = 2.0


class QmpThread(threading.Thread):
    def __init__(self, sock_path):
        super().__init__()
        self.sock_path = sock_path
        self.running = True
        self.error = None

    def run(self):
        try:
            sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            sock.connect(self.sock_path)
            f = sock.makefile("rw")

            # Read greeting
            greeting = f.readline()
            if not greeting:
                raise Exception("No QMP greeting")

            # Send qmp_capabilities
            f.write(json.dumps({"execute": "qmp_capabilities"}) + "\n")
            f.flush()
            f.readline()  # Read response

            while self.running:
                start_time = time.time()
                f.write(json.dumps({"execute": "query-status"}) + "\n")
                f.flush()

                resp = f.readline()
                duration = time.time() - start_time
                if duration > QMP_TIMEOUT_S:
                    raise Exception(f"QMP query-status took {duration:.2f}s (>{QMP_TIMEOUT_S}s). BQL is deadlocked!")

                if not resp:
                    raise Exception("QMP connection closed unexpectedly")

                time.sleep(0.1)

            sock.close()
        except Exception as e:
            self.error = str(e)
            traceback.print_exc()


Q_NUM = 0


def pack_req(delta_ns):
    global Q_NUM
    req = ClockAdvanceReq(delta_ns=delta_ns, mujoco_time_ns=0, quantum_number=Q_NUM)
    Q_NUM += 1
    return req.pack()


def send_query(session, delta_ns, label):
    replies = list(session.get(TOPIC, payload=pack_req(delta_ns), timeout=TIMEOUT_S))
    if not replies:
        raise Exception(f"{label}: TIMEOUT — no reply received")
    reply = replies[0]
    if getattr(reply, "err", None) is not None:
        raise Exception(f"{label}: ERROR reply: {reply.err}")
    if not hasattr(reply, "ok") or reply.ok is None:
        raise Exception(f"{label}: NO 'ok' in reply: {reply}")

    resp = ClockReadyResp.unpack(reply.ok.payload.to_bytes())
    if resp.error_code != 0:
        raise Exception(f"{label}: Reply error_code = {resp.error_code} (1=STALL, 2=ZENOH_ERROR)")

    return True


def main():
    qmp_thread = QmpThread(QMP_SOCK)
    qmp_thread.start()

    if len(sys.argv) <= 2:
        logger.error(f"Usage: {sys.argv[0]} <router_endpoint>")
        sys.exit(1)
    router = sys.argv[2]

    config = zenoh.Config()
    config.insert_json5("connect/endpoints", f'["{router}"]')
    config.insert_json5("scouting/multicast/enabled", "false")
    session = zenoh.open(config)

    try:
        for i in range(3):
            # Sleep to ensure QEMU hits the quantum boundary and blocks in clock_quantum_wait
            time.sleep(1.0)

            if qmp_thread.error:
                logger.error(f"FAIL: QMP Thread Error: {qmp_thread.error}")
                sys.exit(1)

            logger.info(f"Sending clock advance {i + 1}...")
            send_query(session, 1_000_000, f"Q{i + 1}")
            logger.info(f"Clock advance {i + 1} OK")

    finally:
        qmp_thread.running = False
        qmp_thread.join(timeout=2.0)
        session.close()

    if qmp_thread.error:
        logger.error(f"FAIL: QMP Thread Error: {qmp_thread.error}")
        sys.exit(1)

    logger.info("PASS")


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    main()
