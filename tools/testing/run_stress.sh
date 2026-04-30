#!/bin/bash
set -euo pipefail

if [ "$#" -lt 1 ]; then
    echo "Usage: $0 <pytest_target> [iterations]"
    echo "Example: $0 tests/test_flexray.py::test_flexray_stress 20"
    exit 1
fi

TEST_TARGET="${1}"
ITERATIONS="${2:-20}"

echo "============================================================"
echo "==> Stress Testing: $TEST_TARGET"
echo "==> Iterations: $ITERATIONS"
echo "============================================================"

# Ensure we are running from the workspace root
cd "$(dirname "$0")/../.."

for i in $(seq 1 "$ITERATIONS"); do
    echo "===== RUN $i of $ITERATIONS ====="
    
    # Run the test. We use --tb=short for cleaner output on success, 
    # but the full failure will print if it crashes.
    PYTHONPATH=$(pwd) pytest "$TEST_TARGET" -v --tb=short -s || {
        echo "❌ FAILED on iteration $i"
        exit 1
    }
done

echo "✅ All $ITERATIONS iterations passed successfully!"
