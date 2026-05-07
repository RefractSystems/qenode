#!/usr/bin/env bash
# ==============================================================================
# scripts/run-miri.sh - Unified Miri Test Runner
#
# Runs Miri to detect Undefined Behavior in pure-Rust logic.
# ==============================================================================
set -euo pipefail

# Find workspace root and change to it
CUR_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(cd "$CUR_DIR/../.." && pwd)"
cd "$WORKSPACE_DIR"

echo "════════════════════════════════════════════════════"
echo "  Running Miri Tests"
echo "════════════════════════════════════════════════════"

# Ensure miri is installed
if ! rustup component list --toolchain nightly 2>/dev/null | grep -q "miri.*(installed)"; then
    echo "==> Installing Rust nightly toolchain and Miri..."
    rustup toolchain install nightly --profile minimal --component miri
fi

cargo +nightly miri setup
MIRIFLAGS="-Zmiri-disable-isolation" cargo +nightly miri test

echo "✓ Miri tests passed."
