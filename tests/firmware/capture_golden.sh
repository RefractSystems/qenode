#!/usr/bin/env bash
# Capture golden UART output from a VirtMCU run for a given firmware target.
# Run this when QEMU is available (inside the Docker build environment).
#
# Usage: ./tests/firmware/capture_golden.sh <target>
#   target: subdirectory under tests/firmware/ (e.g. cortex-a15-virt)
#
# The script boots the firmware in standalone QEMU, captures UART output for
# 3 seconds, strips carriage returns, and writes to golden_uart.txt.
# Output is NOT silicon-validated — update PROVENANCE.md accordingly.

set -euo pipefail

TARGET="${1:?usage: $0 <target>}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FIRMWARE_DIR="$SCRIPT_DIR/$TARGET"
WORKSPACE="$SCRIPT_DIR/../.."

ELF="$FIRMWARE_DIR/echo.elf"
DTB="$WORKSPACE/tests/fixtures/guest_apps/boot_arm/minimal.dtb"
GOLDEN="$FIRMWARE_DIR/golden_uart.txt"
UART_LOG="$(mktemp)"

if [[ ! -f "$ELF" ]]; then
    echo "ERROR: $ELF not found" >&2
    exit 1
fi

QEMU="${QEMU_SYSTEM_ARM:-qemu-system-arm}"

echo "==> Booting $TARGET in standalone mode, capturing UART for 3s..."
timeout 3 "$QEMU" \
    -machine arm-generic-fdt \
    -hw-dtb "$DTB" \
    -kernel "$ELF" \
    -serial file:"$UART_LOG" \
    -display none \
    -nographic \
    2>/dev/null || true

# Strip \r and trim trailing blank lines
sed 's/\r//' "$UART_LOG" | sed '/^[[:space:]]*$/d' > "$GOLDEN"
rm -f "$UART_LOG"

echo "==> Captured to $GOLDEN:"
cat "$GOLDEN"
echo ""
echo "NOTE: This is a VirtMCU-baseline capture. Update PROVENANCE.md if silicon-validated."
