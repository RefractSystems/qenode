"""
Zenoh mock router for TCP connectivity testing.

Listens on tcp/127.0.0.1:7447 with multicast scouting disabled.  QEMU must
connect via `-device zenoh-clock,router=tcp/127.0.0.1:7447,...`.  Once QEMU
registers the sim/clock/advance/0 queryable, the mock completes one full
clock-advance handshake and exits 0.

Multicast is explicitly disabled so the test fails if QEMU ignores the
router= property and falls back to multicast peer discovery.
"""

import struct
import sys
import time

import zenoh

DELTA_NS = 1_000_000  # 1 ms — enough to let the timer fire quickly
TOPIC = "sim/clock/advance/0"
TIMEOUT_S = 15.0


def pack_req(delta_ns: int) -> bytes:
    return struct.pack("<QQ", delta_ns, 0)


def unpack_rep(data: bytes) -> int:
    vtime_ns, _n_frames = struct.unpack("<QI", data)
    return vtime_ns


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
    while time.time() < deadline:
        try:
            payload = pack_req(DELTA_NS)
            replies = list(session.get(TOPIC, payload=payload, timeout=2.0))
            if replies:
                reply = replies[0]
                if hasattr(reply, "ok") and reply.ok is not None:
                    vtime = unpack_rep(reply.ok.payload.to_bytes())
                    print(f"Zenoh TCP connectivity test PASSED! vtime={vtime} ns")
                    session.close()
                    sys.exit(0)
        except Exception as exc:
            print(f"Retry: {exc}")

        time.sleep(0.5)

    print("Timeout: QEMU did not connect via TCP router within 15 s")
    session.close()
    sys.exit(1)


if __name__ == "__main__":
    main()
