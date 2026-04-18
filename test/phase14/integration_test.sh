#!/usr/bin/env bash
# test/phase14/integration_test.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
PORT=7448
ROUTER_ENDPOINT="tcp/127.0.0.1:$PORT"

# Manually point to local build artifacts due to nested install structure
BUNDLE_ROOT="$WORKSPACE_DIR/third_party/qemu/build-virtmcu/qemu-bundle/workspace/third_party/qemu/build-virtmcu/install"
export QEMU_BIN="$BUNDLE_ROOT/bin/qemu-system-arm"
export QEMU_MODULE_DIR="$BUNDLE_ROOT/lib/aarch64-linux-gnu/qemu"

echo "==> Building Phase 14 Radio Test Firmware"
make -C "$SCRIPT_DIR" radio_test.elf

echo "==> Starting Zenoh Router"
python3 "$SCRIPT_DIR/zenoh_router.py" "$PORT" &
ROUTER_PID=$!

# Add router property to board.yaml (where node: 0 is present)
sed -i "s|node: 0|node: 0\n      router: \"$ROUTER_ENDPOINT\"|g" "$SCRIPT_DIR/board.yaml"
sed -i "s|node: \"0\"|node: \"0\"\n      router: \"$ROUTER_ENDPOINT\"|g" "$SCRIPT_DIR/board.yaml"

cleanup() {
    echo "Cleaning up..."
    kill $QEMU_PID 2>/dev/null || true
    kill $ROUTER_PID 2>/dev/null || true
    kill $RESPONDER_PID 2>/dev/null || true
    # Revert board.yaml change
    sed -i "\|router: \"$ROUTER_ENDPOINT\"|d" "$SCRIPT_DIR/board.yaml"
}
trap cleanup EXIT

echo "==> Generating DTB"
python3 -m tools.yaml2qemu "$SCRIPT_DIR/board.yaml" --out-dtb "$SCRIPT_DIR/test.dtb" --out-cli "$SCRIPT_DIR/test.cli"

echo "==> Starting Radio Responder"
python3 "$SCRIPT_DIR/radio_determinism.py" 0 "$ROUTER_ENDPOINT" &
RESPONDER_PID=$!

echo "==> Running QEMU with Radio Test Firmware"
# Use the local binary directly
echo "Running: $QEMU_BIN -M arm-generic-fdt,hw-dtb=$SCRIPT_DIR/test.dtb -kernel $SCRIPT_DIR/radio_test.elf ..."
$QEMU_BIN \
    -M arm-generic-fdt,hw-dtb="$SCRIPT_DIR/test.dtb" \
    -kernel "$SCRIPT_DIR/radio_test.elf" \
    -nographic \
    -serial stdio \
    -monitor none \
    -icount shift=0,align=off,sleep=off \
    2>&1 | tee "$SCRIPT_DIR/output.log" &
QEMU_PID=$!

echo "Waiting for test to complete (timeout 20s)..."
count=0
while true; do
    if [ -f "$SCRIPT_DIR/output.log" ] && grep -q "MATCHED ACK" "$SCRIPT_DIR/output.log"; then
        echo "SUCCESS: Radio test completed and received MATCHED ACK"
        if grep -q "MISMATCHED ACK" "$SCRIPT_DIR/output.log"; then
            echo "FAILED: Received MISMATCHED ACK (Filter failed!)"
            exit 1
        fi
        echo "✓ Filter verified: MISMATCHED ACK was correctly dropped."
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
