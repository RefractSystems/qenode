#!/usr/bin/env bash
# ==============================================================================
# Smoke Test — RISC-V Expansion
#
# Verifies that the unified run.sh pipeline can detect a RISC-V DTS, select
# qemu-system-riscv64, and boot a minimal RISC-V firmware that prints to UART.
#
# Test flow:
#   1. Build the RISC-V firmware + DTB from tests/fixtures/guest_apps/boot_riscv/.
#   2. Run QEMU via run.sh with a 5-second timeout (firmware loops after output).
#   3. Capture serial output in a temp file and assert "HI RV" is present.
# ==============================================================================

set -euo pipefail

echo "=============================================================================="
echo "🧪 RUNNING TEST: $(basename "$0")"
echo "=============================================================================="
cat << 'TEST_DOC_BLOCK'
==============================================================================
Smoke Test — RISC-V Expansion

Verifies that the unified run.sh pipeline can detect a RISC-V DTS, select
qemu-system-riscv64, and boot a minimal RISC-V firmware that prints to UART.

Test flow:
  1. Build the RISC-V firmware + DTB from tests/fixtures/guest_apps/boot_riscv/.
  2. Run QEMU via run.sh with a 5-second timeout (firmware loops after output).
  3. Capture serial output in a temp file and assert "HI RV" is present.
==============================================================================
TEST_DOC_BLOCK
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
RISCV_TEST_DIR="$WORKSPACE_DIR/tests/fixtures/guest_apps/boot_riscv"
OUTPUT_LOG=$(mktemp /tmp/riscv_complex-uart-XXXXXX.log)
trap 'rm -f "$OUTPUT_LOG"' EXIT

echo "==> Running Smoke Test (RISC-V Expansion)..."

# Ensure the firmware and DTB are built
make -C "$RISCV_TEST_DIR"

# Under ASan, QEMU is significantly slower. Scale the timeout accordingly.
TIMEOUT="5s"
if [ "${VIRTMCU_USE_ASAN:-0}" = "1" ]; then
    TIMEOUT="30s"
fi

echo "==> Booting RISC-V firmware ($TIMEOUT timeout)..."

# Run QEMU with a hard timeout.  The firmware prints "HI RV" then enters a WFI
# loop, so QEMU will never exit on its own — the timeout is expected behaviour.
# -serial file: captures UART output; -monitor none suppresses the QEMU monitor.
timeout "$TIMEOUT" "$RUN_SH" \
    --dts "$RISCV_TEST_DIR/minimal.dts" \
    --kernel "$RISCV_TEST_DIR/hello.elf" \
    -nographic \
    -monitor none \
    -serial "file:$OUTPUT_LOG" \
    || true   # timeout exits 124; treat as success so we can inspect output

echo "==> Serial output captured:"
cat "$OUTPUT_LOG"

if grep -q "HI RV" "$OUTPUT_LOG"; then
    echo "✓ Smoke Test PASSED: RISC-V firmware printed 'HI RV'."
    exit 0
else
    echo "✗ Smoke Test FAILED: 'HI RV' not found in serial output."
    exit 1
fi
