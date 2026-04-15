#!/usr/bin/env bash
# ==============================================================================
# smoke_test.sh (Phase 4)
#
# This script validates the QMP-based testing infrastructure. It executes
# the pytest suite for the QmpBridge and the Robot Framework integration tests.
# ==============================================================================

set -e

echo "=============================================================================="
echo "🧪 RUNNING TEST: $(basename "$0")"
echo "=============================================================================="
cat << 'TEST_DOC_BLOCK'
==============================================================================
smoke_test.sh (Phase 4)

This script validates the QMP-based testing infrastructure. It executes
the pytest suite for the QmpBridge and the Robot Framework integration tests.
==============================================================================
TEST_DOC_BLOCK
echo "=============================================================================="


SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(dirname "$(dirname "$SCRIPT_DIR")")"

echo "Running Phase 4 smoke test (QMP Bridge & Keywords)..."

# 0. Ensure firmware is built
if [ ! -f "$WORKSPACE_DIR/test/phase1/hello.elf" ]; then
    echo "Building Phase 1 firmware..."
    make -C "$WORKSPACE_DIR/test/phase1" > /dev/null
fi

# Set PYTHONPATH so tests can find the tools package
export PYTHONPATH="$WORKSPACE_DIR"

# 1. Run pytest suite
echo "1. Running pytest suite (QmpBridge)..."
pytest "$WORKSPACE_DIR/tools/testing/test_qmp.py" -v

# 2. Run Robot Framework suite
echo "2. Running Robot Framework suite (qemu_keywords.robot)..."
robot --outputdir "$SCRIPT_DIR/results" "$WORKSPACE_DIR/tests/test_qmp_keywords.robot"

echo "Phase 4 smoke test: PASSED"
