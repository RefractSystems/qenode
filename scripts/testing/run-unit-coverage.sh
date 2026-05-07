#!/usr/bin/env bash
# ==============================================================================
# scripts/testing/run-unit-coverage.sh - Unified Unit Test Coverage Runner
#
# Generates code coverage reports for Python and Rust unit tests.
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
echo "  Running Unit Tests with Coverage (Python + Rust)"
echo "════════════════════════════════════════════════════"

mkdir -p test-results/
mkdir -p coverage-data/

# 1. Python Unit Coverage
echo "==> Running Python Unit Tests Coverage..."
bash scripts/cleanup-sim.sh --quiet

PYTEST_WORKERS=${PYTEST_WORKERS:-auto}

# Note: Requires pytest-cov to be installed
if [ "$INSIDE_DOCKER" = "true" ]; then
    pytest tests/unit/ -v -n "$PYTEST_WORKERS" --tb=short \
        --cov=tools --cov=tests/unit --cov-report=xml:test-results/python-unit-coverage.xml --cov-report=term
else
    uv run --active pytest tests/unit/ -v -n "$PYTEST_WORKERS" --tb=short \
        --cov=tools --cov=tests/unit --cov-report=xml:test-results/python-unit-coverage.xml --cov-report=term
fi
echo "✓ Python unit coverage generated."

# 2. Rust Unit Coverage
echo "==> Running Rust Unit Tests Coverage..."
if command -v cargo-tarpaulin >/dev/null 2>&1; then
    # Redirect LLVM profiling data to coverage-data/ to avoid root clutter
    export LLVM_PROFILE_FILE="$WORKSPACE_DIR/coverage-data/rust-unit-%m-%p.profraw"

    # We use the same link-arg workaround as run-unit.sh for isolated plugin testing
    VIRTMCU_UNIT_TEST=1 QEMU_SRC_DIR=/nonexistent VIRTMCU_SKIP_QEMU_HEADERS_WARNING=1 \
    RUSTFLAGS="-C link-arg=-Wl,--unresolved-symbols=ignore-all --cfg virtmcu_unit_test" \
    cargo tarpaulin --workspace --out Xml --output-dir test-results/ \
        --ignore-tests --engine llvm
    
    # Rename to be explicit
    mv test-results/cobertura.xml test-results/rust-unit-coverage.xml 2>/dev/null || true
    echo "✓ Rust unit coverage generated."
else
    echo "⚠ Skipping Rust unit coverage (cargo-tarpaulin not installed)."
    echo "   Run cargo install cargo-tarpaulin to enable."
fi

echo "✓ All unit coverage tasks completed."
