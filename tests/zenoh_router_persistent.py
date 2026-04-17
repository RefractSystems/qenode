import sys
import time

import zenoh

DEFAULT_ENDPOINT = "tcp/127.0.0.1:7447"


def main():
    endpoint = sys.argv[1] if len(sys.argv) > 1 else DEFAULT_ENDPOINT
    config = zenoh.Config()
    config.insert_json5("listen/endpoints", f'["{endpoint}"]')
    config.insert_json5("scouting/multicast/enabled", "false")
    print(f"Starting persistent Zenoh mock router on {endpoint}...")
    session = zenoh.open(config)
    try:
        while True:
            time.sleep(1)
    except KeyboardInterrupt:
        pass
    session.close()


if __name__ == "__main__":
    main()
