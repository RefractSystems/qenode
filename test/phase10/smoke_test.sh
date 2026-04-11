#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

echo "[phase10] Building tools/cyber_bridge..."
mkdir -p "$WORKSPACE_DIR/tools/cyber_bridge/build"
cd "$WORKSPACE_DIR/tools/cyber_bridge/build"
cmake .. > /dev/null
make > /dev/null

echo "[phase10] Running OpenUSD Metadata Tool..."
"$WORKSPACE_DIR/tools/usd_to_virtmcu.py" "$WORKSPACE_DIR/test/phase3/test_board.yaml" > /tmp/board.hpp
grep -q "MEMORY_BASE" /tmp/board.hpp

echo "[phase10] Creating dummy RESD file..."
# Write RESD magic, version (1), padding (3)
printf "RESD\x01\x00\x00\x00" > /tmp/dummy.resd
# Block Header: block_type(0x01 ARBITRARY), sample_type(0x0002 ACCELERATION), channel_id(0), data_size(16)
printf "\x01\x02\x00\x00\x00\x10\x00\x00\x00\x00\x00\x00\x00" >> /tmp/dummy.resd
# Subheader: start_time (0)
printf "\x00\x00\x00\x00\x00\x00\x00\x00" >> /tmp/dummy.resd
# Metadata: size(0)
printf "\x00\x00\x00\x00\x00\x00\x00\x00" >> /tmp/dummy.resd

echo "[phase10] Running resd_replay with dummy file..."
./resd_replay /tmp/dummy.resd 0 > /tmp/resd.log &
REPLAY_PID=$!
sleep 1
kill $REPLAY_PID || true

grep -q "Connected. Acting as TimeAuthority" /tmp/resd.log

echo "[phase10] Running mujoco_bridge..."
./mujoco_bridge > /tmp/mujoco.log
grep -q "Ready to synchronize Zenoh Clock" /tmp/mujoco.log

echo "=== Phase 10 smoke test PASSED ==="
