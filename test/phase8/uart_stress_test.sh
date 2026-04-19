#!/usr/bin/env bash
set -euo pipefail

echo "=============================================================================="
echo "🧪 RUNNING TEST: $(basename "$0")"
echo "=============================================================================="

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
TMPDIR_LOCAL="$(mktemp -d /tmp/uart_stress_XXXXXX)"
QEMU_PID=""
ROUTER_PID=""
PORT=7448

cleanup() {
    echo "Cleaning up..."
    [[ -n "${QEMU_PID:-}" ]] && kill -9 "$QEMU_PID" 2>/dev/null || true
    [[ -n "${ROUTER_PID:-}" ]] && kill -9 "$ROUTER_PID" 2>/dev/null || true
    # rm -rf "$TMPDIR_LOCAL"
}
trap cleanup EXIT

echo "TMPDIR: $TMPDIR_LOCAL"

# Start Zenoh Router on a unique port
python3 -u "$WORKSPACE_DIR/tests/zenoh_router_persistent.py" "tcp/127.0.0.1:$PORT" &
ROUTER_PID=$!
sleep 2

# Start QEMU in slaved-icount mode
# Using minimal.dtb from phase1
"$WORKSPACE_DIR/scripts/run.sh" --dtb "$WORKSPACE_DIR/test/phase1/minimal.dtb" \
    -kernel "$WORKSPACE_DIR/test/phase8/echo.elf" \
    -icount shift=0,align=off,sleep=off \
    -device zenoh-clock,node=0,mode=slaved-icount,router=tcp/127.0.0.1:$PORT,stall-timeout=60000 \
    -chardev zenoh,id=uart0,node=0,router=tcp/127.0.0.1:$PORT \
    -serial chardev:uart0 \
    -d in_asm \
    -display none -monitor none > "$TMPDIR_LOCAL/qemu.log" 2>&1 &
QEMU_PID=$!

sleep 2

# Run Stress Test
if python3 "$WORKSPACE_DIR/test/phase8/uart_stress_test.py" "tcp/127.0.0.1:$PORT"; then
    echo "=== Phase 8 UART Stress Test PASSED ==="
else
    echo "=== Phase 8 UART Stress Test FAILED ==="
    echo "--- QEMU LOG ---"
    cat "$TMPDIR_LOCAL/qemu.log"
    echo "-----------------"
    exit 1
fi
