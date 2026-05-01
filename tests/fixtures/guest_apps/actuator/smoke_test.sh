#!/usr/bin/env bash
set -euo pipefail

# tests/fixtures/guest_apps/actuator/smoke_test.sh

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

echo "[actuator] Building firmware..."
make -C "$SCRIPT_DIR" clean > /dev/null
make -C "$SCRIPT_DIR" > /dev/null

echo "[actuator] Generating DTB..."
export PYTHONPATH=${PYTHONPATH:-}:$WORKSPACE_DIR
uv run python3 -m tools.yaml2qemu "$SCRIPT_DIR/board.yaml" --out-dtb "$SCRIPT_DIR/board.dtb" > /dev/null

echo "[actuator] Running verification script..."
pytest tests/integration/peripherals/test_actuator.py

echo "[actuator] PASSED"
