#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

source "$WORKSPACE_DIR/.venv/bin/activate"

echo "==> Building Zenoh Coordinator"
export PATH="$HOME/.cargo/bin:$PATH"
(cd "$WORKSPACE_DIR/tools/zenoh_coordinator" && cargo build)

echo "==> Running Interactive Echo Test"
export PYTHONPATH="$WORKSPACE_DIR"
robot --outputdir "$WORKSPACE_DIR/test/phase8/results" "$WORKSPACE_DIR/tests/test_interactive_echo.robot"
