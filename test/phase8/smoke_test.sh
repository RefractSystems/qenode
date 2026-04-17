#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

echo "==> Building Zenoh Coordinator"
# Ensure cargo is in PATH
if [ -f "$HOME/.cargo/env" ]; then
    # shellcheck source=/dev/null
    source "$HOME/.cargo/env"
fi
(cd "$WORKSPACE_DIR/tools/zenoh_coordinator" && cargo build --release)

# Ensure Phase 1 artifacts are built as they are used by the Robot test
if [ ! -f "$WORKSPACE_DIR/test/phase1/minimal.dtb" ] || [ ! -f "$WORKSPACE_DIR/test/phase1/hello.elf" ]; then
    echo "Phase 1 artifacts not found. Building Phase 1 first..."
    make -C "$WORKSPACE_DIR/test/phase1"
fi

echo "==> Running Interactive Echo Test"
export PYTHONPATH="$WORKSPACE_DIR"
# robot is expected to be installed in the system environment
robot --outputdir "$WORKSPACE_DIR/test/phase8/results" "$WORKSPACE_DIR/tests/test_interactive_echo.robot"
