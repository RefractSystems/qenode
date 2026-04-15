
import zenoh
import time
import sys

def main():
    config = zenoh.Config()
    config.insert_json5("listen/endpoints", '["tcp/127.0.0.1:7447"]')
    config.insert_json5("scouting/multicast/enabled", "false")
    print("Starting persistent Zenoh mock router on tcp/127.0.0.1:7447...")
    session = zenoh.open(config)
    try:
        while True:
            time.sleep(1)
    except KeyboardInterrupt:
        pass
    session.close()

if __name__ == "__main__":
    main()
