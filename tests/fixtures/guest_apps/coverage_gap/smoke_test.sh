#!/bin/bash
# ==============================================================================
# Smoke Test: Distribution & Packaging
#
# Verifies that the virtmcu-tools package can be built and installed,
# and that the resulting CLI tools are functional.
# ==============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"
PACKAGING_DIR="$REPO_ROOT/packaging/virtmcu-tools"
TEST_VENV="$REPO_ROOT/.test_venv_coverage_gap"

echo "==> Verifying virtmcu-tools packaging..."

# 1. Build the package
echo "--> Building virtmcu-tools wheel..."
cd "$PACKAGING_DIR"
uv build --wheel

WHEEL_FILE=$(ls dist/*.whl | head -n 1)
echo "✓ Wheel built: $WHEEL_FILE"

# 2. Create a clean virtual environment and install the wheel
echo "--> Testing installation in a clean venv..."
rm -rf "$TEST_VENV"
uv venv "$TEST_VENV"
source "$TEST_VENV/bin/activate"

uv pip install "$WHEEL_FILE"

# 3. Verify CLI tools
echo "--> Verifying CLI tools..."

if ! repl2qemu --help > /dev/null; then
    echo "FAILED: repl2qemu --help failed"
    exit 1
fi
echo "✓ repl2qemu is functional."

if ! yaml2qemu --help > /dev/null; then
    echo "FAILED: yaml2qemu --help failed"
    exit 1
fi
echo "✓ yaml2qemu is functional."

# Check if the command exists
if ! command -v virtmcu-mcp > /dev/null; then
    echo "FAILED: virtmcu-mcp command not found"
    exit 1
fi
echo "✓ virtmcu-mcp command found."

# 4. End-to-end test: run yaml2qemu on a sample board
echo "--> Testing yaml2qemu end-to-end..."
cd "$REPO_ROOT"
mkdir -p tests/fixtures/guest_apps/coverage_gap/tmp
source "$TEST_VENV/bin/activate"
yaml2qemu docs/tutorials/lesson15-distribution/src/board.yaml --out-dtb tests/fixtures/guest_apps/coverage_gap/tmp/board.dtb

if [ ! -f tests/fixtures/guest_apps/coverage_gap/tmp/board.dtb ]; then
    echo "FAILED: yaml2qemu did not produce board.dtb"
    exit 1
fi
echo "✓ yaml2qemu successfully generated DTB."

# Cleanup
rm -rf "$TEST_VENV"
rm -rf tests/fixtures/guest_apps/coverage_gap/tmp

echo "✓ smoke test passed."
