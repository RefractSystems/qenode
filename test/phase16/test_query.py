import sys
import time
from pathlib import Path

import zenoh
from vproto import ClockAdvanceReq, ClockReadyResp

# Add tools/ to path
SCRIPT_DIR = Path(Path(__file__).resolve().parent)
WORKSPACE_DIR = Path(Path(SCRIPT_DIR).parent.parent)
sys.path.append(str(Path(WORKSPACE_DIR) / "tools"))


def main():
    router = sys.argv[1] if len(sys.argv) > 1 else "tcp/127.0.0.1:7447"

    config = zenoh.Config()
    config.insert_json5("connect/endpoints", f'["{router}"]')
    config.insert_json5("scouting/multicast/enabled", "false")
    session = zenoh.open(config)

    topic = "sim/clock/advance/0"
    print(f"Sending query to {topic}...")

    req = ClockAdvanceReq(delta_ns=1000000, mujoco_time_ns=0).pack()

    start = time.perf_counter()
    replies = list(session.get(topic, payload=req, timeout=10.0))
    end = time.perf_counter()

    if not replies:
        print("No replies received!")
    else:
        for reply in replies:
            if reply.ok:
                resp = ClockReadyResp.unpack(reply.ok.payload.to_bytes())
                print(f"Reply: vtime={resp.current_vtime_ns}, error={resp.error_code}")
            else:
                print(f"Error reply: {reply.err}")

    print(f"Query took {end - start:.3f}s")
    session.close()


if __name__ == "__main__":
    main()
