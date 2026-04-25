#!/usr/bin/env bash
# ==============================================================================
# smoke_test.sh (Phase 3.5)
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
smoke_test.sh (Phase 3.5)

This script tests the virtmcu YAML hardware description pipeline. It parses
a .yaml file, generates a DTB, and verifies that QEMU can boot and print "HI".
==============================================================================
TEST_DOC_BLOCK
echo "=============================================================================="


SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(dirname "$(dirname "$SCRIPT_DIR")")"
RUN_SH="$WORKSPACE_DIR/scripts/run.sh"

YAML_FILE="$WORKSPACE_DIR/test/phase3/test_board.yaml"
KERNEL="$WORKSPACE_DIR/test/phase1/hello.elf"

if [ ! -f "$YAML_FILE" ]; then
    echo "FAILED: Test .yaml file not found at $YAML_FILE"
    exit 1
fi

if [ ! -f "$KERNEL" ]; then
    echo "Kernel not found. Building Phase 1 first..."
    make -C "$WORKSPACE_DIR/test/phase1"
fi

echo "Running Phase 3.5 smoke test (YAML platform parsing)..."

# 1. Boot QEMU with the YAML platform directly
echo "1. Booting QEMU via run.sh --yaml $YAML_FILE"
OUTPUT_LOG=$(mktemp /tmp/smoke_test_3.5-XXXXXX.log)
trap 'rm -f "$OUTPUT_LOG"' EXIT
rm -f "$OUTPUT_LOG"

timeout 3s "$RUN_SH" --yaml "$YAML_FILE" --kernel "$KERNEL" -nographic -monitor none -serial file:"$OUTPUT_LOG" || true

# 2. Verification
if [ -f "$OUTPUT_LOG" ] && grep -q "HI" "$OUTPUT_LOG"; then
    echo "Phase 3.5 smoke test: PASSED (Kernel printed 'HI' via YAML description)"
    exit 0
else
    echo "Phase 3.5 smoke test: FAILED (No 'HI' detected)"
    if [ -f "$OUTPUT_LOG" ]; then
        echo "--- QEMU LOG ---"
        cat "$OUTPUT_LOG"
    fi
    exit 1
fi
