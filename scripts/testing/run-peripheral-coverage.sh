#!/usr/bin/env bash
set -euo pipefail

# Argument 1: Path to coverage data directory (defaults to /workspace/all-coverage for CI)
COV_DIR="${1:-/workspace/all-coverage}"

mkdir -p /workspace/test-results
# Ensure the directory exists to avoid gcovr error if no artifacts found
mkdir -p "$COV_DIR"

# Find source and build directories
VIRTMCU_SRC=$(find /build/qemu/hw/virtmcu third_party/qemu/hw/virtmcu -maxdepth 0 2>/dev/null | head -n 1 || true)
if [ -z "$VIRTMCU_SRC" ]; then
    echo "❌ Error: virtmcu source directory not found"
    exit 1
fi

QEMU_BUILD=$(find /build/qemu/build-virtmcu third_party/qemu/build-virtmcu -maxdepth 0 2>/dev/null | head -n 1 || true)
if [ -z "$QEMU_BUILD" ]; then
    echo "❌ Error: QEMU build directory not found"
    exit 1
fi

echo "==> Running gcovr against $COV_DIR..."
echo "==> Source: $VIRTMCU_SRC"
echo "==> Build: $QEMU_BUILD"

gcovr -r "$VIRTMCU_SRC" \
    --gcov-executable gcov \
    --gcov-ignore-errors=no_working_dir_found \
    --object-directory "$QEMU_BUILD" \
    --xml /workspace/test-results/peripheral-coverage.xml \
    --html-details /workspace/test-results/peripheral-coverage.html \
    --print-summary \
    "$COV_DIR"
