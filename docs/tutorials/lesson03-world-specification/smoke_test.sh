#!/usr/bin/env bash
# ==============================================================================
# smoke_test.sh (yaml_boot)
#
# This script tests the repl2qemu toolchain. It parses a Renode .repl file,
# generates a QEMU Device Tree Blob (.dtb), and verifies that QEMU can boot
# the arm-generic-fdt machine using the generated DTB.
# ==============================================================================

set -euo pipefail

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

REPL_FILE="$SCRIPT_DIR/src/test_board.repl"
OUT_DTB="$SCRIPT_DIR/test_board_out.dtb"
KERNEL="$WORKSPACE_DIR/tests/fixtures/guest_apps/boot_arm/hello.elf"

if [ ! -f "$REPL_FILE" ]; then
    echo "FAILED: Test .repl file not found at $REPL_FILE"
    exit 1
fi

if [ ! -f "$KERNEL" ]; then
    echo "Kernel not found. Building boot_arm first..."
    make -C "$WORKSPACE_DIR/tests/fixtures/guest_apps/boot_arm"
fi

echo "Running Lesson 3 smoke test (repl2qemu parser)..."

# Ensure clean state
rm -f "$OUT_DTB" qemu_lesson3.log

# 1. Run the repl2qemu parser using the Python module invocation
echo "1. Parsing $REPL_FILE -> $OUT_DTB"
python3 -m tools.repl2qemu "$REPL_FILE" --out-dtb "$OUT_DTB"

if [ ! -f "$OUT_DTB" ]; then
    echo "FAILED: Parser did not generate $OUT_DTB"
    exit 1
fi

# 2. Boot QEMU with the generated DTB and the boot_arm bare-metal kernel
echo "2. Booting QEMU with generated DTB..."

# We run with the boot_arm kernel which prints "HI" to the PL011 UART
# Under ASan, QEMU is significantly slower. Scale the timeout accordingly.
TIMEOUT="2s"
if [ "${VIRTMCU_USE_ASAN:-0}" = "1" ]; then
    TIMEOUT="20s"
fi

timeout "$TIMEOUT" "$RUN_SH" --dtb "$OUT_DTB" \
    --kernel "$KERNEL" \
    -nographic \
    -monitor none \
    -m 128M \
    -serial file:qemu_lesson3.log || true

# 3. Verification
# If the repl was translated correctly to a DTB, the kernel will successfully boot and print "HI"
if grep -q "HI" qemu_lesson3.log; then
    echo "Lesson 3 smoke test: PASSED"
    rm -f "$OUT_DTB" qemu_lesson3.log "${OUT_DTB}.dts"
    exit 0
else
    echo "Lesson 3 smoke test: FAILED (No 'HI' detected)"
    echo "--- QEMU LOG ---"
    cat qemu_lesson3.log
    exit 1
fi
