#!/usr/bin/env bash
# ==============================================================================
# scripts/testing/run-unit.sh - Unified Unit Test Runner
#
# Runs both Python and Rust unit tests. These tests are fast and do not
# require a running QEMU instance.
# ==============================================================================
set -euo pipefail

# SOTA: Source common helpers (sets WORKSPACE_DIR, SCRIPTS_DIR, PYTHONPATH, etc.)
# shellcheck source=scripts/common.sh
source "$(dirname "${BASH_SOURCE[0]}")/../common.sh"

cd "$WORKSPACE_DIR"

# Detect if we are inside docker
INSIDE_DOCKER=false
if [ -f /.dockerenv ] || grep -q "docker" /proc/1/cgroup 2>/dev/null; then
    INSIDE_DOCKER=true
fi

echo "════════════════════════════════════════════════════"
echo "  Running Unit Tests (Python + Rust)"
echo "════════════════════════════════════════════════════"

# 1. Python Unit Tests
echo "==> Running Python Unit Tests..."
bash scripts/cleanup-sim.sh --quiet

# Parallelism for pytest (default to auto, can be overridden)
PYTEST_WORKERS=${PYTEST_WORKERS:-auto}

# If inside docker, we might not have 'uv' or might want to use system python
if [ "$INSIDE_DOCKER" = "true" ]; then
    pytest tests/unit/ -v -n "$PYTEST_WORKERS" --tb=short --capture=sys
else
    uv run pytest tests/unit/ -v -n "$PYTEST_WORKERS" --tb=short --capture=sys
fi

# 2. Rust Unit Tests
echo "==> Running Rust Unit Tests..."
# We use link-arg=-Wl,--unresolved-symbols=ignore-all because we are testing
# the plugins in isolation without the QEMU main binary symbols.
VIRTMCU_UNIT_TEST=1 QEMU_SRC_DIR=/nonexistent VIRTMCU_SKIP_QEMU_HEADERS_WARNING=1 \
    RUSTFLAGS="-C link-arg=-Wl,--unresolved-symbols=ignore-all --cfg virtmcu_unit_test" \
    cargo test --workspace

echo "✓ All unit tests passed."
