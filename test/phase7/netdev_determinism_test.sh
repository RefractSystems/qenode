#!/usr/bin/env bash
# test/phase7/netdev_determinism_test.sh — Determinism test for zenoh-netdev
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "Running netdev determinism test..."
python3 "$SCRIPT_DIR/netdev_determinism_test.py"
echo "✓ Netdev determinism test PASSED"
