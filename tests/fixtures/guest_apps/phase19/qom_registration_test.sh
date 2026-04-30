#!/bin/bash
set -euo pipefail

SUFFIX="$( [ "${VIRTMCU_USE_ASAN:-0}" = "1" ] && echo "-asan" || echo "" )"

echo "Rebuilding QEMU to include test-qom-device..."
make -C third_party/qemu/build-virtmcu$SUFFIX install

echo "Running QEMU to list devices..."
# Set QEMU_MODULE_DIR to the directory where we just installed the modules
QEMU_MODULE_DIR=$(find third_party/qemu/build-virtmcu$SUFFIX/install -name "hw-virtmcu-*.so" | head -n1 | xargs dirname)
export QEMU_MODULE_DIR
if third_party/qemu/build-virtmcu$SUFFIX/install/bin/qemu-system-arm -device help 2>&1 | grep -q "test-rust-device"; then
    echo "SUCCESS: test-rust-device found!"
    exit 0
else
    echo "FAILED: test-rust-device not found in QEMU help."
    third_party/qemu/build-virtmcu$SUFFIX/install/bin/qemu-system-arm -device help 2>&1 | grep test || true
    exit 1
fi
