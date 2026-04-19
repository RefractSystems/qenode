#!/usr/bin/env bash
# test/phase6/smoke_test.sh — Phase 6 smoke test: Zenoh Multi-Node Coordinator
#
# Verifies:
#   1. The Rust zenoh_coordinator process builds and starts successfully.
#   2. Basic forwarding (ETH, UART, SystemC, RF).
#   3. Robustness against malformed packets.
#   4. Virtual time overflow handling (saturating add).
#   5. Topology control (link delay and drop probability).

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
TMPDIR_LOCAL="$(mktemp -d /tmp/phase6_XXXXXX)"

COORD_PID=0
ROUTER_PID=0

cleanup() {
    echo "Cleaning up..."
    [[ $COORD_PID -ne 0 ]] && kill "$COORD_PID" 2>/dev/null || true
    [[ $ROUTER_PID -ne 0 ]] && kill "$ROUTER_PID" 2>/dev/null || true
    rm -rf "$TMPDIR_LOCAL"
}
trap cleanup EXIT

echo "Starting Zenoh Router..."
python3 -u "$WORKSPACE_DIR/tests/zenoh_router_persistent.py" &
ROUTER_PID=$!
sleep 1

# Ensure Zenoh components connect to our router
export ZENOH_CONNECT="tcp/127.0.0.1:7447"
export ZENOH_MULTICAST_SCOUTING="false"

echo "Building Zenoh Coordinator..."
if [ -f "$HOME/.cargo/env" ]; then
    # shellcheck source=/dev/null
    source "$HOME/.cargo/env"
fi
cd "$WORKSPACE_DIR/tools/zenoh_coordinator"
cargo build --release > /dev/null 2>&1

echo "Starting Zenoh Coordinator (with --sensitivity=-70.0)..."
target/release/zenoh_coordinator --sensitivity=-70.0 > "$TMPDIR_LOCAL/coord.log" 2>&1 &
COORD_PID=$!

sleep 2 # Let coordinator initialize its Zenoh subscriber

if ! kill -0 $COORD_PID 2>/dev/null; then
    echo "FAIL: Coordinator failed to start!"
    cat "$TMPDIR_LOCAL/coord.log"
    exit 1
fi

echo "Running comprehensive Phase 6 test suite..."
python3 "$WORKSPACE_DIR/test/phase6/complete_test.py"

echo "Running malformed packet survival test..."
python3 "$WORKSPACE_DIR/test/phase6/repro_crash.py"

echo "Running stress test (20 nodes, 1000 messages)..."
python3 "$WORKSPACE_DIR/test/phase6/stress_test.py"

if ! kill -0 $COORD_PID 2>/dev/null; then
    echo "FAIL: Coordinator crashed during tests!"
    cat "$TMPDIR_LOCAL/coord.log"
    exit 1
fi

echo "=== Phase 6 tests PASSED ==="
exit 0
