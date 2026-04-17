#!/usr/bin/env bash
# test/phase16/smoke_test.sh — Performance & Determinism CI
set -euo pipefail

echo "=============================================================================="
echo "🧪 RUNNING TEST: $(basename "$0")"
echo "=============================================================================="

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Ensure benchmark firmware is built
make -C "$SCRIPT_DIR"

# Run the benchmark script
# We pass the threshold as an environment variable if needed
python3 "$SCRIPT_DIR/bench.py"

echo "=== Phase 16 smoke test PASSED ==="
