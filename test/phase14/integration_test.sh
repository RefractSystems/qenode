#!/usr/bin/env bash
# test/phase14/integration_test.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
PORT=$(python3 "$WORKSPACE_DIR/scripts/get-free-port.py")
ROUTER_ENDPOINT="tcp/127.0.0.1:$PORT"

# Cleanup on exit
cleanup() {
    echo "Cleaning up..."
    # Kill all background jobs
    local pids
    pids=$(jobs -p)
    if [ -n "$pids" ]; then
        # shellcheck disable=SC2086
        kill $pids 2>/dev/null || true
    fi
    rm -f "$SCRIPT_DIR/output.log" "$SCRIPT_DIR/test.dtb" "$SCRIPT_DIR/test.cli" "$SCRIPT_DIR/ack_received.tmp" "$SCRIPT_DIR/board_local.yaml"
}
trap cleanup EXIT

# 0. Clean old state
rm -f "$SCRIPT_DIR/ack_received.tmp"

# 1. Build firmware
echo "==> Building Phase 14 Radio Test Firmware"
make -C "$SCRIPT_DIR" -q || make -C "$SCRIPT_DIR"

# 2. Start Zenoh Router
echo "==> Starting Zenoh Router"
python3 "$SCRIPT_DIR/zenoh_router.py" $PORT > /dev/null 2>&1 &
sleep 2

# 3. Generate DTB and CLI args
echo "==> Generating DTB"
sed "s|tcp/127.0.0.1:7448|$ROUTER_ENDPOINT|g" "$SCRIPT_DIR/board.yaml" > "$SCRIPT_DIR/board_local.yaml"
python3 -m tools.yaml2qemu "$SCRIPT_DIR/board_local.yaml" --out-dtb "$SCRIPT_DIR/test.dtb" --out-cli "$SCRIPT_DIR/test.cli"

# 4. Start Radio Responder
echo "==> Starting Radio Responder"
export PYTHONPATH="$WORKSPACE_DIR"
python3 "$SCRIPT_DIR/radio_determinism.py" 0 "$ROUTER_ENDPOINT" &
sleep 2

# 5. Run QEMU
echo "==> Running QEMU with Radio Test Firmware"
BUNDLE_ROOT="$WORKSPACE_DIR/third_party/qemu/build-virtmcu/install"
QEMU_BIN="$BUNDLE_ROOT/bin/qemu-system-arm"
export QEMU_MODULE_DIR="$BUNDLE_ROOT/lib/aarch64-linux-gnu/qemu"

# We override -serial to see output on stdout for easier verification
$QEMU_BIN -M arm-generic-fdt,hw-dtb="$SCRIPT_DIR/test.dtb" \
    -kernel "$SCRIPT_DIR/radio_test.elf" \
    -nographic -serial stdio -monitor none \
    -icount shift=0,align=off,sleep=off \
    2>&1 | tee "$SCRIPT_DIR/output.log" &

echo "Waiting for test to complete (timeout 20s)..."
count=0
while true; do
    if grep -q "Received packet!" "$SCRIPT_DIR/output.log" && \
       grep -q "MATCHED ACK" "$SCRIPT_DIR/output.log"; then
        echo "SUCCESS: Radio test completed and received MATCHED ACK"
        
        # Verify Filter
        if grep -q "MISMATCHED ACK" "$SCRIPT_DIR/output.log"; then
            echo "FAILED: Received MISMATCHED ACK (Filter failed!)"
            exit 1
        else
            echo "✓ Filter verified: MISMATCHED ACK was correctly dropped."
        fi

        # Verify Auto-ACK
        # Wait a bit for the responder to write the file
        sleep 1
        if [ -f "$SCRIPT_DIR/ack_received.tmp" ]; then
            echo "✓ Auto-ACK verified: QEMU sent back an ACK frame."
        else
            echo "FAILED: QEMU did not send back an ACK frame."
            exit 1
        fi

        exit 0
    fi
    
    sleep 1
    count=$((count + 1))
    if [ $count -gt 20 ]; then
        echo "TIMEOUT: Radio test did not receive MATCHED ACK"
        echo "--- QEMU Output ---"
        cat "$SCRIPT_DIR/output.log" || true
        exit 1
    fi
done
