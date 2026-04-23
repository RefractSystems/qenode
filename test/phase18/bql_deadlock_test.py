import json
import socket
import sys
import threading
import time
import traceback
from pathlib import Path

import zenoh

SCRIPT_DIR = Path(Path(__file__).resolve().parent)
TOOLS_DIR = str(Path(Path(SCRIPT_DIR).parent.parent) / "tools")
if TOOLS_DIR not in sys.path:
    sys.path.append(TOOLS_DIR)

from vproto import ClockAdvanceReq, ClockReadyResp  # noqa: E402

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


def pack_req(delta_ns):
    req = ClockAdvanceReq(delta_ns=delta_ns, mujoco_time_ns=0)
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

    router = sys.argv[2] if len(sys.argv) > 2 else "tcp/127.0.0.1:7447"

    config = zenoh.Config()
    config.insert_json5("connect/endpoints", f'["{router}"]')
    config.insert_json5("scouting/multicast/enabled", "false")
    session = zenoh.open(config)

    try:
        for i in range(3):
            # Sleep to ensure QEMU hits the quantum boundary and blocks in zenoh_clock_quantum_wait
            time.sleep(1.0)

            if qmp_thread.error:
                print(f"FAIL: QMP Thread Error: {qmp_thread.error}", file=sys.stderr)
                sys.exit(1)

            print(f"Sending clock advance {i + 1}...")
            send_query(session, 1_000_000, f"Q{i + 1}")
            print(f"Clock advance {i + 1} OK")

    finally:
        qmp_thread.running = False
        qmp_thread.join(timeout=2.0)
        session.close()

    if qmp_thread.error:
        print(f"FAIL: QMP Thread Error: {qmp_thread.error}", file=sys.stderr)
        sys.exit(1)

    print("PASS")


if __name__ == "__main__":
    main()
