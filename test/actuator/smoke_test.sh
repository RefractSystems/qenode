#!/usr/bin/env bash
set -euo pipefail

# test/actuator/smoke_test.sh

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

echo "[actuator] Building firmware..."
make -C "$SCRIPT_DIR" clean > /dev/null
make -C "$SCRIPT_DIR" > /dev/null

echo "[actuator] Generating DTB..."
export PYTHONPATH=${PYTHONPATH:-}:$WORKSPACE_DIR
python3 -m tools.yaml2qemu "$SCRIPT_DIR/board.yaml" --out-dtb "$SCRIPT_DIR/board.dtb" > /dev/null

echo "[actuator] Running verification script..."
uv run python3 "$SCRIPT_DIR/verify_control.py"

echo "[actuator] PASSED"
