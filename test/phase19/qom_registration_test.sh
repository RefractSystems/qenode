#!/bin/bash
set -ex

echo "Rebuilding QEMU to include test-qom-device..."
make -C third_party/qemu/build-virtmcu install

echo "Running QEMU to list devices..."
if third_party/qemu/build-virtmcu/install/bin/qemu-system-arm -device help 2>&1 | grep -q "test-rust-device"; then
    echo "SUCCESS: test-rust-device found!"
    exit 0
else
    echo "FAILED: test-rust-device not found in QEMU help."
    third_party/qemu/build-virtmcu/install/bin/qemu-system-arm -device help 2>&1 | grep test
    exit 1
fi
