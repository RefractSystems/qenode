import zenoh
import time
import sys

def main():
    # Configure to listen on a specific port
    config = zenoh.Config()
    config.insert_json5("listen/endpoints", '["tcp/127.0.0.1:7447"]')
    
    print("Starting Zenoh mock router on tcp/127.0.0.1:7447...")
    session = zenoh.open(config)
    
    # We want to see if someone connects and registers sim/clock/advance/0
    print("Waiting for QEMU to register sim/clock/advance/0...")
    
    start_time = time.time()
    timeout = 10 # seconds
    
    while time.time() - start_time < timeout:
        # Check if the queryable exists
        # We can do a 'get' with a short timeout
        try:
            # We use a very short timeout for the get itself
            replies = session.get("sim/clock/advance/0", zenoh.Queue(), timeout=1)
            # If we get here without exception, it might just mean no one replied yet
            # but the get was sent. 
            # Actually, session.get returns an iterable of replies.
            for reply in replies:
                print(f"Received reply from: {reply.key_expr}")
                print("✅ Zenoh connectivity test PASSED!")
                session.close()
                sys.exit(0)
        except Exception:
            pass
        
        time.sleep(0.5)

    print("❌ Error: Timeout waiting for QEMU to connect to Zenoh router")
    session.close()
    sys.exit(1)

if __name__ == "__main__":
    main()
