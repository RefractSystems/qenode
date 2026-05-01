#!/usr/bin/env bash
# ==============================================================================
# Integration Test
#
# This script spins up the MCP server and a mock client, provisions a board,
# flashes a minimal firmware, runs it, and reads its CPU state via MCP.
# ==============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# Find workspace root (robustly)
_search_dir="$SCRIPT_DIR"
while [[ "$_search_dir" != "/" ]]; do
    if [[ -f "$_search_dir/scripts/common.sh" ]]; then
        source "$_search_dir/scripts/common.sh"
        break
    fi
    _search_dir=$(dirname "$_search_dir")
done

if [[ -z "${WORKSPACE_DIR:-}" ]]; then
    echo "ERROR: Could not find scripts/common.sh" >&2
    exit 1
fi

echo "Running MCP Server test..."

# First, ensure test firmware exists
make -C "$WORKSPACE_DIR/tests/fixtures/guest_apps/boot_arm"

# Run the mock client
python3 "$SCRIPT_DIR/mock_mcp_client.py"

echo "Running Multi-node MCP test..."
python3 "$SCRIPT_DIR/multi_node_mcp_test.py"

echo "Running MCP stress test..."
python3 "$SCRIPT_DIR/mcp_stress_test.py"

echo "Running validation test..."
python3 "$SCRIPT_DIR/validation_test.py"

echo "tests passed!"
