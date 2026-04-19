import os
import sys

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.append(os.path.join(SCRIPT_DIR, "telemetry_fbs"))

import zenoh  # noqa: E402
from Virtmcu.Telemetry.TraceEvent import TraceEvent  # noqa: E402


def on_sample(sample):
    payload = sample.payload.to_bytes()
    try:
        ev = TraceEvent.GetRootAs(payload, 0)

        ts = ev.TimestampNs()
        ev_type = ev.Type()
        ev_id = ev.Id()
        val = ev.Value()
        name = ev.DeviceName()
        if name:
            name_str = name.decode("utf-8")
        else:
            name_str = ""

        if ev_type == 0:  # CPU_STATE
            type_str = "CPU_STATE"
            id_str = f"cpu={ev_id}"
        elif ev_type == 1:  # IRQ
            type_str = "IRQ"
            slot = ev_id >> 16
            pin = ev_id & 0xFFFF
            id_str = f"slot={slot:2} pin={pin:2}"
            if name_str:
                id_str += f" ({name_str})"
        elif ev_type == 2:  # PERIPHERAL
            type_str = "PERIPHERAL"
            id_str = f"id={ev_id}"
        else:
            type_str = "UNKNOWN"
            id_str = f"id={ev_id}"

        print(f"[{ts:15}] {type_str:10} {id_str} val={val:3}")
    except Exception as e:
        print(f"Received malformed payload of size {len(payload)}: {payload.hex()} ({e})")


def main():
    import argparse

    parser = argparse.ArgumentParser(description="Zenoh Telemetry Listener")
    parser.add_argument("node_id", nargs="?", default="0", help="Node ID to listen for")
    parser.add_argument("--router", help="Zenoh router endpoint")
    args = parser.parse_args()

    topic = f"sim/telemetry/trace/{args.node_id}"
    print(f"Listening on {topic}...")

    conf = zenoh.Config()
    if args.router:
        conf.insert_json5("mode", '"client"')
        conf.insert_json5("connect/endpoints", f'["{args.router}"]')

    session = zenoh.open(conf)
    _sub = session.declare_subscriber(topic, on_sample)

    try:
        import time

        while True:
            time.sleep(1)
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
