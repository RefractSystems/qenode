#!/usr/bin/env bash
# ==============================================================================
# smoke_test.sh (Phase 2)
#
# This script performs a basic integration test for Phase 2.
# It launches QEMU with the `dummy-device` loaded dynamically via `-device`,
# starts a QMP server, and uses a Python script to assert that the module
# correctly registered and instantiated itself into the QOM tree.
# ==============================================================================

set -e

echo "=============================================================================="
echo "🧪 RUNNING TEST: $(basename "$0")"
echo "=============================================================================="
cat << 'TEST_DOC_BLOCK'
==============================================================================
smoke_test.sh (Phase 2)

This script performs a basic integration test for Phase 2.
It launches QEMU with the `dummy-device` loaded dynamically via `-device`,
starts a QMP server, and uses a Python script to assert that the module
correctly registered and instantiated itself into the QOM tree.
==============================================================================
TEST_DOC_BLOCK
echo "=============================================================================="


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

echo "Running Phase 2 smoke test (Dynamic Plugins)..."

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
    > qemu_phase2.log 2>&1 &

QEMU_PID=$!

# Wait briefly for socket
sleep 1

# Run the python verification script
if python3 "$CHECK_SCRIPT"; then
    echo "Phase 2 smoke test: PASSED"
    RET=0
else
    echo "Phase 2 smoke test: FAILED"
    echo "--- QEMU LOG ---"
    cat qemu_phase2.log
    RET=1
fi

# Clean up
kill ${QEMU_PID:-} 2>/dev/null || true
rm -f qmp.sock qemu_phase2.log

exit $RET
