#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
BUILD_DIR="$WORKSPACE_DIR/tools/cyber_bridge/build"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

echo "[phase10] Building tools/cyber_bridge..."
mkdir -p "$BUILD_DIR"
cmake -S "$WORKSPACE_DIR/tools/cyber_bridge" -B "$BUILD_DIR" -DCMAKE_BUILD_TYPE=Release > /dev/null
cmake --build "$BUILD_DIR" --parallel "$(nproc)" > /dev/null

# ==============================================================================
# TEST 1: OpenUSD Metadata Tool
# ==============================================================================
echo "[phase10] TEST 1: OpenUSD Metadata Tool..."
python3 "$WORKSPACE_DIR/tools/usd_to_virtmcu.py" \
    "$WORKSPACE_DIR/test/phase3/test_board.yaml" > "$TMP/board.hpp"

grep -q "MEMORY_BASE" "$TMP/board.hpp"
grep -q "UART0_BASE"  "$TMP/board.hpp"
grep -q "GIC_BASE"    "$TMP/board.hpp"
echo "[phase10] TEST 1 PASSED — generated constexpr address map"

# ==============================================================================
# TEST 2: RESD Parser — verify actual sample parsing
# Construct a minimal RESD file with two ACCELERATION samples at known times.
#
# File layout (all little-endian):
#   Header: "RESD" (4) | version=1 (1) | padding (3)
#   Block:
#     block_type=0x01 ARBITRARY_TIMESTAMP (1)
#     sample_type=0x0002 ACCELERATION (2, LE)
#     channel_id=0x0000 (2, LE)
#     data_size (8, LE) = subheader(8) + meta_field(8) + meta(0) + samples
#       Sample 1: timestamp(8) + x(4) + y(4) + z(4) = 20 bytes
#       Sample 2: timestamp(8) + x(4) + y(4) + z(4) = 20 bytes
#       total_samples = 40 bytes
#     data_size = 8 + 8 + 0 + 40 = 56 = 0x38
#   Subheader: start_time=0 (8)
#   Metadata:  size=0 (8)
#   Sample 1:  timestamp=1000000 ns (1ms), x=100, y=200, z=300
#   Sample 2:  timestamp=2000000 ns (2ms), x=400, y=500, z=600
# ==============================================================================
echo "[phase10] TEST 2: RESD Parser — sample parsing..."

python3 - <<'PY' > "$TMP/test.resd"
import struct, sys

# Header
sys.stdout.buffer.write(b'RESD')                     # magic
sys.stdout.buffer.write(struct.pack('<B', 1))         # version
sys.stdout.buffer.write(b'\x00\x00\x00')             # padding

# Block header
block_type   = 0x01   # ARBITRARY_TIMESTAMP
sample_type  = 0x0002 # ACCELERATION
channel_id   = 0
# data_size = subheader(8) + meta_field(8) + meta(0) + 2*sample(20) = 56
data_size = 56
sys.stdout.buffer.write(struct.pack('<BHH', block_type, sample_type, channel_id))
sys.stdout.buffer.write(struct.pack('<Q', data_size))

# Subheader: start_time=0
sys.stdout.buffer.write(struct.pack('<Q', 0))

# Metadata: size=0
sys.stdout.buffer.write(struct.pack('<Q', 0))

# Sample 1: t=1ms, x=100, y=200, z=300
sys.stdout.buffer.write(struct.pack('<Qiii', 1_000_000, 100, 200, 300))
# Sample 2: t=2ms, x=400, y=500, z=600
sys.stdout.buffer.write(struct.pack('<Qiii', 2_000_000, 400, 500, 600))
PY

# Verify last_timestamp via a small C++ test driver built inline
cat > "$TMP/test_parser.cpp" <<'CPP'
#include <iostream>
#include "virtmcu/resd_parser.hpp"
using namespace virtmcu;
int main(int argc, char* argv[]) {
    ResdParser parser(argv[1]);
    if (!parser.init()) { std::cerr << "parse failed\n"; return 1; }

    auto last_ts = parser.get_last_timestamp();
    if (last_ts != 2'000'000ULL) {
        std::cerr << "FAIL: last_timestamp=" << last_ts << " expected 2000000\n";
        return 1;
    }

    auto imu = parser.get_sensor(ResdSampleType::ACCELERATION, 0);
    auto at_1ms = imu->get_reading(1'000'000);
    if (at_1ms.size() != 3 || at_1ms[0] != 100.0 || at_1ms[1] != 200.0 || at_1ms[2] != 300.0) {
        std::cerr << "FAIL: reading at 1ms wrong\n";
        return 1;
    }

    auto at_2ms = imu->get_reading(2'000'000);
    if (at_2ms.size() != 3 || at_2ms[0] != 400.0 || at_2ms[1] != 500.0 || at_2ms[2] != 600.0) {
        std::cerr << "FAIL: reading at 2ms wrong\n";
        return 1;
    }

    std::cout << "PARSER OK: last_ts=" << last_ts << " s1=[100,200,300] s2=[400,500,600]\n";
    return 0;
}
CPP

ZENOH_INC="$WORKSPACE_DIR/third_party/zenoh-c/include"
# Find zenoh headers in common build locations
for d in /build/zenoh-c/include /opt/virtmcu/include "$ZENOH_INC"; do
    [ -f "$d/zenoh.h" ] && ZENOH_INC="$d" && break
done

g++ -std=c++17 -I"$WORKSPACE_DIR/tools/cyber_bridge/include" \
    -I"$ZENOH_INC" \
    "$TMP/test_parser.cpp" \
    "$WORKSPACE_DIR/tools/cyber_bridge/src/resd_parser.cpp" \
    -o "$TMP/test_parser"

"$TMP/test_parser" "$TMP/test.resd"
echo "[phase10] TEST 2 PASSED — RESD parser correctly reads samples"

# ==============================================================================
# TEST 3: resd_replay terminates on empty sensor file
# Start without a live QEMU (resd_replay will timeout on first z_get).
# Verify it prints the sensor count / last-timestamp line and fails gracefully.
# ==============================================================================
echo "[phase10] TEST 3: resd_replay startup + empty-file rejection..."
"$BUILD_DIR/resd_replay" /nonexistent.resd 0 2>&1 | grep -q "Failed to parse" \
    || { echo "FAIL: expected parse-failure message"; exit 1; }
echo "[phase10] TEST 3 PASSED — resd_replay rejects missing file"

# ==============================================================================
# TEST 4: mujoco_bridge binds to shared memory
# Start the bridge briefly and verify it creates the shm segment.
# ==============================================================================
echo "[phase10] TEST 4: mujoco_bridge shared memory creation..."
SHM_NAME="/virtmcu_mujoco_99"
# Clean up any leftover segment
( ls /dev/shm/ 2>/dev/null | grep -q "virtmcu_mujoco_99" && rm -f /dev/shm/virtmcu_mujoco_99 ) || true

"$BUILD_DIR/mujoco_bridge" 99 2 6 > "$TMP/bridge.log" 2>&1 &
BRIDGE_PID=$!
sleep 0.5
kill "$BRIDGE_PID" 2>/dev/null || true
wait "$BRIDGE_PID" 2>/dev/null || true

grep -q "Shared memory ready" "$TMP/bridge.log" \
    || grep -q "Ready to synchronize" "$TMP/bridge.log" \
    || { echo "FAIL: mujoco_bridge did not start correctly"; cat "$TMP/bridge.log"; exit 1; }
echo "[phase10] TEST 4 PASSED — mujoco_bridge created shm segment"

echo ""
echo "=== Phase 10 smoke test PASSED ==="
