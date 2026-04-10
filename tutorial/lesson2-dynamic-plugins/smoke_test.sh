#!/usr/bin/env bash
# ==============================================================================
# smoke_test.sh (Lesson 2)
#
# This script performs a basic integration test for Lesson 2.
# It launches QEMU with the `dummy-device` loaded dynamically via `-device`,
# starts a QMP server, and uses a Python script to assert that the module
# correctly registered and instantiated itself into the QOM tree.
# ==============================================================================

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(dirname "$(dirname "$SCRIPT_DIR")")"
RUN_SH="$WORKSPACE_DIR/scripts/run.sh"
CHECK_SCRIPT="$SCRIPT_DIR/check_dummy_qmp.py"

# Use the DTB from phase 1 as a base
DTB="$WORKSPACE_DIR/test/phase1/minimal.dtb"

if [ ! -f "$DTB" ]; then
    echo "DTB not found. Building Phase 1 first..."
    make -C "$WORKSPACE_DIR/test/phase1"
fi

echo "Running Lesson 2 smoke test (Dynamic Plugins)..."

# Ensure socket is clear
rm -f qmp.sock

# Launch QEMU with the device and a QMP server.
# We don't need a kernel for this, QEMU will idle or exit after a CPU fault,
# but we append `|| true` and manage its lifecycle directly.
"$RUN_SH" --dtb "$DTB" \
    -device dummy-device \
    -nographic \
    -monitor none \
    -qmp unix:qmp.sock,server,nowait \
    -m 128M \
    > qemu_lesson2.log 2>&1 &

QEMU_PID=$!

# Run the python verification script
# (check_dummy_qmp.py retries the socket connection for up to 5 seconds internally)
if python3 "$CHECK_SCRIPT"; then
    echo "Lesson 2 smoke test: PASSED"
    RET=0
else
    echo "Lesson 2 smoke test: FAILED"
    echo "--- QEMU LOG ---"
    cat qemu_lesson2.log
    RET=1
fi

# Clean up
kill $QEMU_PID 2>/dev/null || true
rm -f qmp.sock qemu_lesson2.log

exit $RET