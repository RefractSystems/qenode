#!/usr/bin/env bash
# ==============================================================================
# Phase 11 Smoke Test — RISC-V Expansion
#
# Verifies that the unified run.sh pipeline can detect a RISC-V DTS, select
# qemu-system-riscv64, and boot a minimal RISC-V firmware that prints to UART.
#
# Test flow:
#   1. Build the RISC-V firmware + DTB from tests/fixtures/guest_apps/riscv/.
#   2. Run QEMU via run.sh with a 5-second timeout (firmware loops after output).
#   3. Capture serial output in a temp file and assert "HI RV" is present.
# ==============================================================================

set -euo pipefail

echo "=============================================================================="
echo "🧪 RUNNING TEST: $(basename "$0")"
echo "=============================================================================="
cat << 'TEST_DOC_BLOCK'
==============================================================================
Phase 11 Smoke Test — RISC-V Expansion

Verifies that the unified run.sh pipeline can detect a RISC-V DTS, select
qemu-system-riscv64, and boot a minimal RISC-V firmware that prints to UART.

Test flow:
  1. Build the RISC-V firmware + DTB from tests/fixtures/guest_apps/riscv/.
  2. Run QEMU via run.sh with a 5-second timeout (firmware loops after output).
  3. Capture serial output in a temp file and assert "HI RV" is present.
==============================================================================
TEST_DOC_BLOCK
echo "=============================================================================="


SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(dirname "$(dirname "$SCRIPT_DIR")")"
RISCV_TEST_DIR="$WORKSPACE_DIR/tests/fixtures/guest_apps/riscv"
RUN_SH="$WORKSPACE_DIR/scripts/run.sh"
OUTPUT_LOG=$(mktemp /tmp/phase11-uart-XXXXXX.log)
trap 'rm -f "$OUTPUT_LOG"' EXIT

echo "==> Running Phase 11 Smoke Test (RISC-V Expansion)..."

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
    echo "✓ Phase 11 Smoke Test PASSED: RISC-V firmware printed 'HI RV'."
    exit 0
else
    echo "✗ Phase 11 Smoke Test FAILED: 'HI RV' not found in serial output."
    exit 1
fi
