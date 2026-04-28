"""
Zenoh mock router for TCP connectivity testing.

Listens on tcp/127.0.0.1:7447 with multicast scouting disabled.  QEMU must
connect via `-device virtmcu-clock,router=tcp/127.0.0.1:7447,...`.  Once QEMU
registers the sim/clock/advance/0 queryable, the mock completes one full
clock-advance handshake and exits 0.

Multicast is explicitly disabled so the test fails if QEMU ignores the
router= property and falls back to multicast peer discovery.
"""

import sys
import time
from pathlib import Path

SCRIPT_DIR = Path(Path(__file__).resolve().parent)
TOOLS_DIR = Path(Path(SCRIPT_DIR).parent) / "tools"
if str(TOOLS_DIR) not in sys.path:
    sys.path.append(str(TOOLS_DIR))

import zenoh  # noqa: E402
from vproto import ClockAdvanceReq, ClockReadyResp  # noqa: E402

DELTA_NS = 1_000_000  # 1 ms — enough to let the timer fire quickly
TOPIC = "sim/clock/advance/0"
TIMEOUT_S = 15.0


def pack_req(delta_ns: int) -> bytes:
    req = ClockAdvanceReq(delta_ns=delta_ns, mujoco_time_ns=0, quantum_number=0)
    return req.pack()


def unpack_rep(data: bytes) -> int:
    resp = ClockReadyResp.unpack(data)
    return resp.current_vtime_ns


def main() -> None:
    config = zenoh.Config()
    # Listen on a fixed TCP port so QEMU can connect via router=tcp/...
    config.insert_json5("listen/endpoints", '["tcp/127.0.0.1:7447"]')
    # Disable multicast — if QEMU ignores router= it won't reach us, test fails
    config.insert_json5("scouting/multicast/enabled", "false")

    print("Starting Zenoh mock router on tcp/127.0.0.1:7447...")
    session = zenoh.open(config)

    print(f"Waiting for QEMU to register {TOPIC}...")

    deadline = time.time() + TIMEOUT_S
    attempt = 0
    while time.time() < deadline:
        attempt += 1
        try:
            payload = pack_req(DELTA_NS)
            print(f"Attempt {attempt}: Sending GET to {TOPIC}...")
            replies = list(session.get(TOPIC, payload=payload, timeout=2.0))
            if replies:
                print(f"Attempt {attempt}: Received {len(replies)} replies")
                reply = replies[0]
                if hasattr(reply, "ok") and reply.ok is not None:
                    vtime = unpack_rep(reply.ok.payload.to_bytes())
                    print(f"Zenoh TCP connectivity test PASSED! vtime={vtime} ns")
                    session.close()
                    sys.exit(0)
                else:
                    print(f"Attempt {attempt}: Received reply but it is not 'ok': {reply}")
            else:
                print(f"Attempt {attempt}: No replies received")
        except Exception as exc:
            print(f"Attempt {attempt}: Exception during GET: {exc}")

        time.sleep(0.5)

    print("Timeout: QEMU did not connect via TCP router within 15 s")
    session.close()
    sys.exit(1)


if __name__ == "__main__":
    main()
