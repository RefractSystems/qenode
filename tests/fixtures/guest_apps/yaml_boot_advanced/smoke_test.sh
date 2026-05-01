#!/usr/bin/env bash
# ==============================================================================
# smoke_test.sh (yaml_boot_advanced)
#
# This script tests the virtmcu YAML hardware description pipeline. It parses 
# a .yaml file, generates a DTB, and verifies that QEMU can boot and print "HI".
# ==============================================================================

set -euo pipefail

echo "=============================================================================="
echo "🧪 RUNNING TEST: $(basename "$0")"
echo "=============================================================================="
cat << 'TEST_DOC_BLOCK'
==============================================================================
smoke_test.sh (yaml_boot_advanced)

This script tests the virtmcu YAML hardware description pipeline. It parses
a .yaml file, generates a DTB, and verifies that QEMU can boot and print "HI".
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

YAML_FILE="$WORKSPACE_DIR/tests/fixtures/guest_apps/yaml_boot/test_board.yaml"
KERNEL="$WORKSPACE_DIR/tests/fixtures/guest_apps/boot_arm/hello.elf"

if [ ! -f "$YAML_FILE" ]; then
    echo "FAILED: Test .yaml file not found at $YAML_FILE"
    exit 1
fi

if [ ! -f "$KERNEL" ]; then
    echo "Kernel not found. Building first..."
    make -C "$WORKSPACE_DIR/tests/fixtures/guest_apps/boot_arm"
fi

echo "Running smoke test (YAML platform parsing)..."

# 1. Boot QEMU with the YAML platform directly
echo "1. Booting QEMU via run.sh --yaml $YAML_FILE"
OUTPUT_LOG=$(mktemp /tmp/smoke_test_3.5-XXXXXX.log)
trap 'rm -f "$OUTPUT_LOG"' EXIT
rm -f "$OUTPUT_LOG"

# Under ASan, QEMU is significantly slower. Scale the timeout accordingly.
TIMEOUT="3s"
if [ "${VIRTMCU_USE_ASAN:-0}" = "1" ]; then
    TIMEOUT="20s"
fi

timeout "$TIMEOUT" "$RUN_SH" --yaml "$YAML_FILE" --kernel "$KERNEL" -nographic -monitor none -serial file:"$OUTPUT_LOG" || true

# 2. Verification
if [ -f "$OUTPUT_LOG" ] && grep -q "HI" "$OUTPUT_LOG"; then
    echo "smoke test: PASSED (Kernel printed 'HI' via YAML description)"
    exit 0
else
    echo "smoke test: FAILED (No 'HI' detected)"
    if [ -f "$OUTPUT_LOG" ]; then
        echo "--- QEMU LOG ---"
        cat "$OUTPUT_LOG"
    fi
    exit 1
fi
