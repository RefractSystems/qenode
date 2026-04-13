#!/usr/bin/env bash
# ==============================================================================
# Phase 11 Smoke Test — RISC-V Expansion
#
# This test verifies that the QEMU RISC-V machine can be constructed via the
# dynamic machine pipeline (FDT) and successfully boot a RISC-V firmware.
# ==============================================================================

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(dirname "$(dirname "$SCRIPT_DIR")")"
RISCV_TEST_DIR="$WORKSPACE_DIR/test/riscv"

echo "==> Running Phase 11 Smoke Test (RISC-V Expansion)..."

# Ensure the firmware and DTB are built
make -C "$RISCV_TEST_DIR"

# Run QEMU with the RISC-V firmware and capture output
echo "==> Booting RISC-V firmware..."
OUTPUT=$("$WORKSPACE_DIR/scripts/run.sh" \
    --dts "$RISCV_TEST_DIR/minimal.dts" \
    --kernel "$RISCV_TEST_DIR/hello.elf" \
    -nographic \
    -d in_asm \
    -D /tmp/qemu-riscv.log 2>&1)

echo "$OUTPUT"

if echo "$OUTPUT" | grep -q "HI RV"; then
    echo "✓ Phase 11 Smoke Test PASSED: RISC-V firmware successfully executed and output 'HI RV'."
    exit 0
else
    echo "✗ Phase 11 Smoke Test FAILED: Did not find 'HI RV' in output."
    exit 1
fi
