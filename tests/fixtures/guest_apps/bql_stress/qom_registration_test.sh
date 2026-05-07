#!/bin/bash
set -euo pipefail

# SOTA: This test verifies that the 'test-rust-device' (a test-only dynamic QOM plugin)
# is correctly registered and visible to QEMU.

# Find workspace root
CUR_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(cd "$CUR_DIR/../../../.." && pwd)"
cd "$WORKSPACE_DIR"

# If QEMU_BIN is not provided, we use scripts/run.sh which is the project's
# authoritative broker for finding the correct QEMU binary and plugins.
if [ -z "${QEMU_BIN:-}" ]; then
    echo "==> QEMU_BIN not set. Delegating to scripts/run.sh for discovery..."
    if ./scripts/run.sh -device help 2>&1 | grep -q "test-rust-device"; then
        echo "✓ SUCCESS: test-rust-device found via scripts/run.sh"
        exit 0
    else
        echo "❌ FAILED: test-rust-device not found via scripts/run.sh"
        ./scripts/run.sh -device help 2>&1 | grep test || true
        exit 1
    fi
else
    # Use the passed QEMU_BIN (following the user's "pass location" recommendation)
    echo "==> Using provided QEMU_BIN: $QEMU_BIN"
    
    # Ensure QEMU_MODULE_DIR is also set if we are using a custom QEMU,
    # as dynamic plugins depend on it.
    if [ -z "${QEMU_MODULE_DIR:-}" ]; then
        echo "⚠ WARNING: QEMU_BIN provided but QEMU_MODULE_DIR is missing."
    fi

    echo "Running QEMU to list devices..."
    if "$QEMU_BIN" -device help 2>&1 | grep -q "test-rust-device"; then
        echo "✓ SUCCESS: test-rust-device found!"
        exit 0
    else
        echo "❌ FAILED: test-rust-device not found in QEMU help."
        "$QEMU_BIN" -device help 2>&1 | grep test || true
        exit 1
    fi
fi
