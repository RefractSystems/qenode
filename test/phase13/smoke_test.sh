#!/usr/bin/env bash
# ==============================================================================
# Phase 13 Integration Test
#
# This script spins up the MCP server and a mock client, provisions a board,
# flashes a minimal firmware, runs it, and reads its CPU state via MCP.
# ==============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(dirname "$(dirname "$SCRIPT_DIR")")"

echo "Running Phase 13 MCP Server test..."

# First, ensure Phase 1 test firmware exists
make -C "$WORKSPACE_DIR/test/phase1"

# Run the mock client
python3 "$SCRIPT_DIR/mock_mcp_client.py"

echo "Running Multi-node MCP test..."
python3 "$SCRIPT_DIR/multi_node_mcp_test.py"

echo "Running MCP stress test..."
python3 "$SCRIPT_DIR/mcp_stress_test.py"

echo "Running validation test..."
python3 "$SCRIPT_DIR/validation_test.py"

echo "Phase 13 tests passed!"
