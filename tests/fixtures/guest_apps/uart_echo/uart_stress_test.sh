#!/usr/bin/env bash
set -euo pipefail

echo "=============================================================================="
echo "🧪 RUNNING TEST: $(basename "$0")"
echo "=============================================================================="

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# Find workspace root (robustly)
_search_dir="$SCRIPT_DIR"
while [[ "$_search_dir" != "/" ]]; do
    if [[ -f "$_search_dir/scripts/common.sh" ]]; then
        source "$_search_dir/scripts/common.sh"
        break
    fi
    _search_dir=$(dirname "$_search_dir")
done

if [[ -z "${WORKSPACE_DIR:-}" ]]; then
    echo "ERROR: Could not find scripts/common.sh" >&2
    exit 1
fi
TMPDIR_LOCAL="$(mktemp -d /tmp/uart_stress_XXXXXX)"
QEMU_PID=""
ROUTER_PID=""
ROUTER_ENDPOINT=${1:-""}

if [ -z "$ROUTER_ENDPOINT" ]; then
    # Find a free endpoint using python
    ROUTER_ENDPOINT=$(python3 "$SCRIPTS_DIR/get-free-port.py" --endpoint --proto "tcp/")
fi

cleanup() {
    echo "Cleaning up..."
    [[ -n "${QEMU_PID:-}" ]] && kill -9 "$QEMU_PID" 2>/dev/null || true
    [[ -n "${ROUTER_PID:-}" ]] && kill -9 "$ROUTER_PID" 2>/dev/null || true
    rm -rf "$TMPDIR_LOCAL"
}
trap cleanup EXIT

echo "TMPDIR: $TMPDIR_LOCAL"

# Start Zenoh Router on a unique port
python3 -u "$WORKSPACE_DIR/tests/zenoh_router_persistent.py" "$ROUTER_ENDPOINT" &
ROUTER_PID=$!
sleep 2

# Start QEMU in slaved-icount mode
# Using minimal.dtb from uart_echo
"$SCRIPTS_DIR/run.sh" --dtb "$WORKSPACE_DIR/tests/fixtures/guest_apps/boot_arm/minimal.dtb" \
    -kernel "$WORKSPACE_DIR/tests/fixtures/guest_apps/uart_echo/echo.elf" \
    -icount shift=6,align=off,sleep=off \
    -device virtmcu-clock,node=0,mode=slaved-icount,router=$ROUTER_ENDPOINT, \
    -chardev zenoh,id=uart0,node=0,router=$ROUTER_ENDPOINT \
    -serial chardev:uart0 \
    -display none -monitor none > "$TMPDIR_LOCAL/qemu.log" 2>&1 &
QEMU_PID=$!

sleep 2

# Run Stress Test
if python3 "$WORKSPACE_DIR/tests/fixtures/guest_apps/uart_echo/uart_stress_test.py" "$ROUTER_ENDPOINT"; then
    echo "=== UART Stress Test PASSED ==="
else
    echo "=== UART Stress Test FAILED ==="
    echo "--- QEMU LOG ---"
    cat "$TMPDIR_LOCAL/qemu.log"
    echo "-----------------"
    exit 1
fi
