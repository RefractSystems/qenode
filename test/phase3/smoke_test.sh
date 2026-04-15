#!/usr/bin/env bash
# ==============================================================================
# smoke_test.sh (Phase 3)
#
# This script tests the repl2qemu toolchain. It parses a Renode .repl file,
# generates a QEMU Device Tree Blob (.dtb), and verifies that QEMU can boot
# the arm-generic-fdt machine using the generated DTB.
# ==============================================================================

set -e

echo "=============================================================================="
echo "🧪 RUNNING TEST: $(basename "$0")"
echo "=============================================================================="
cat << 'TEST_DOC_BLOCK'
==============================================================================
smoke_test.sh (Phase 3)

This script tests the repl2qemu toolchain. It parses a Renode .repl file,
generates a QEMU Device Tree Blob (.dtb), and verifies that QEMU can boot
the arm-generic-fdt machine using the generated DTB.
==============================================================================
TEST_DOC_BLOCK
echo "=============================================================================="


SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(dirname "$(dirname "$SCRIPT_DIR")")"
RUN_SH="$WORKSPACE_DIR/scripts/run.sh"

REPL_FILE="$SCRIPT_DIR/test_board.repl"
OUT_DTB="$SCRIPT_DIR/test_board_out.dtb"
KERNEL="$WORKSPACE_DIR/test/phase1/hello.elf"

if [ ! -f "$REPL_FILE" ]; then
    echo "FAILED: Test .repl file not found at $REPL_FILE"
    exit 1
fi

if [ ! -f "$KERNEL" ]; then
    echo "Kernel not found. Building Phase 1 first..."
    make -C "$WORKSPACE_DIR/test/phase1"
fi

echo "Running Phase 3 smoke test (repl2qemu parser)..."

# Ensure clean state
rm -f "$OUT_DTB" qemu_phase3.log

# 1. Run the repl2qemu parser as a module
echo "1. Parsing $REPL_FILE -> $OUT_DTB"
python3 -m tools.repl2qemu "$REPL_FILE" --out-dtb "$OUT_DTB"

if [ ! -f "$OUT_DTB" ]; then
    echo "FAILED: Parser did not generate $OUT_DTB"
    exit 1
fi

# 2. Boot QEMU with the generated DTB and the Phase 1 bare-metal kernel
echo "2. Booting QEMU with generated DTB..."

# We run with the Phase 1 kernel which prints "HI" to the PL011 UART
timeout 2s "$RUN_SH" --dtb "$OUT_DTB" \
    --kernel "$KERNEL" \
    -nographic \
    -monitor none \
    -m 128M \
    -serial file:qemu_phase3.log || true

# 3. Verification
# If the repl was translated correctly to a DTB, the kernel will successfully boot and print "HI"
if grep -q "HI" qemu_phase3.log; then
    echo "Phase 3 smoke test: PASSED (Kernel successfully printed 'HI' via translated DTB)"
    rm -f "$OUT_DTB" qemu_phase3.log "${OUT_DTB}.dts"
    exit 0
else
    echo "Phase 3 smoke test: FAILED (No 'HI' detected)"
    echo "--- QEMU LOG ---"
    cat qemu_phase3.log
    exit 1
fi
