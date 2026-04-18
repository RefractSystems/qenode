import zenoh
import sys
import time

def main():
    port = sys.argv[1] if len(sys.argv) > 1 else "7448"
    config = zenoh.Config()
    config.insert_json5("listen/endpoints", f'["tcp/127.0.0.1:{port}"]')
    config.insert_json5("scouting/multicast/enabled", "false")

    print(f"Starting Zenoh router on tcp/127.0.0.1:{port}...")
    session = zenoh.open(config)

    print("Router running. Press Ctrl+C to exit.")
    try:
        while True:
            time.sleep(1)
    except KeyboardInterrupt:
        pass
    session.close()

if __name__ == "__main__":
    main()
