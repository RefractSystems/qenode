#!/usr/bin/env bash
# test/phase6/smoke_test.sh — Phase 6 smoke test: Zenoh Multi-Node Coordinator
#
# Verifies:
#   1. The Rust zenoh_coordinator process builds and starts successfully.
#   2. It accurately subscribes to TX topics and republishes to all known RX topics
#      with the correct delay added to the virtual timestamp.

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
TMPDIR_LOCAL="$(mktemp -d /tmp/phase6_XXXXXX)"

cleanup() {
    kill "${COORD_PID:-}" 2>/dev/null || true
    rm -rf "$TMPDIR_LOCAL"
}
trap cleanup EXIT

echo "Building Zenoh Coordinator..."
if [ -f "$HOME/.cargo/env" ]; then
    # shellcheck source=/dev/null
    source "$HOME/.cargo/env"
fi
cd "$WORKSPACE_DIR/tools/zenoh_coordinator"
cargo build --release > /dev/null 2>&1

echo "Starting Zenoh Coordinator..."
target/release/zenoh_coordinator --delay-ns 100000 > "$TMPDIR_LOCAL/coord.log" 2>&1 &
COORD_PID=$!

sleep 2 # Let coordinator initialize its Zenoh subscriber

cat << 'PY_EOF' > "$TMPDIR_LOCAL/test_coord.py"
import zenoh
import struct
import sys
import time

def main():
    s = zenoh.open(zenoh.Config())
    
    # We will simulate Node 1 and Node 2.
    # We want Node 1 to send a frame, and Node 2 to receive it with +100000ns vtime.
    
    # First, declare a subscriber on Node 2's RX topic
    rx_frames = []
    def on_rx(sample):
        rx_frames.append(sample.payload.to_bytes())
        
    s.declare_subscriber("sim/eth/frame/2/rx", on_rx)
    
    # We also need to declare a subscriber on Node 1's RX topic so the coordinator knows Node 2 exists?
    # Wait, the coordinator learns about nodes when they TRANSMIT on their TX topic.
    # So Node 2 must transmit something first, so the coordinator adds it to known_nodes.
    pub2 = s.declare_publisher("sim/eth/frame/2/tx")
    pub1 = s.declare_publisher("sim/eth/frame/1/tx")
    
    time.sleep(1) # wait for discovery
    
    # Node 2 sends a dummy frame so it becomes "known"
    p2 = struct.pack("<QI", 0, 0) + b""
    pub2.put(p2)
    
    time.sleep(1) # wait for coordinator to process it
    
    # Now Node 1 sends a real frame
    orig_vtime = 5000000
    size = 14
    frame = b'\xaa' * 14
    p1 = struct.pack("<QI", orig_vtime, size) + frame
    
    pub1.put(p1)
    
    # Wait for the coordinator to forward it to Node 2
    deadline = time.time() + 2.0
    while time.time() < deadline:
        if len(rx_frames) > 0:
            break
        time.sleep(0.1)
        
    if len(rx_frames) == 0:
        print("FAIL: No frame received by Node 2", file=sys.stderr)
        sys.exit(1)
        
    data = rx_frames[0]
    vtime, r_size = struct.unpack("<QI", data[:12])
    
    if r_size != size:
        print(f"FAIL: Size mismatch {r_size} != {size}", file=sys.stderr)
        sys.exit(1)
        
    if vtime != orig_vtime + 100000:
        print(f"FAIL: VTime mismatch {vtime} != {orig_vtime + 100000}", file=sys.stderr)
        sys.exit(1)
        
    print("PASS: Frame successfully coordinated and delayed.")
    s.close()

if __name__ == "__main__":
    main()
PY_EOF

echo "Running Python test script..."
python3 "$TMPDIR_LOCAL/test_coord.py"

echo "=== Phase 6 smoke test PASSED ==="
exit 0
