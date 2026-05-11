#!/usr/bin/env bash
# ==============================================================================
# scripts/testing/run-integration-asan.sh - Unified ASan Test Runner
#
# Builds QEMU with ASan/UBSan and runs integration tests.
# ==============================================================================
set -euo pipefail

# Find workspace root and change to it
CUR_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(cd "$CUR_DIR/../.." && pwd)"
cd "$WORKSPACE_DIR"

echo "════════════════════════════════════════════════════"
echo "  Running ASan Integration Tests"
echo "════════════════════════════════════════════════════"

# Detect if we are inside docker
INSIDE_DOCKER=false
if [ -f /.dockerenv ] || grep -q "docker" /proc/1/cgroup 2>/dev/null; then
    INSIDE_DOCKER=true
fi

# Rebuild with ASan if not already done or if forced
# In dev mode, we might want to force it to be sure.
# In CI, we expect the image to be clean or handled.
if [ "${VIRTMCU_USE_PREBUILT_QEMU:-0}" = "1" ]; then
    echo "==> Using pre-built QEMU (skipping build)..."
elif [ "$INSIDE_DOCKER" = "false" ] || [ ! -x "$(which qemu-system-arm 2>/dev/null || echo "$WORKSPACE_DIR/third_party/qemu/build-virtmcu-asan/install/bin/qemu-system-arm")" ]; then
    echo "==> Building QEMU with ASan/UBSan enabled..."
    VIRTMCU_USE_ASAN=1 bash scripts/install-third-party.sh --force
fi

DOMAIN=${1:-all}

echo "==> Running integration tests under ASan/UBSan for domain: $DOMAIN..."
VIRTMCU_USE_ASAN=1 \
VIRTMCU_STALL_TIMEOUT_MS=300000 \
ASAN_OPTIONS=detect_leaks=0,halt_on_error=1,detect_stack_use_after_return=1 \
UBSAN_OPTIONS=halt_on_error=1:print_stacktrace=1 \
make dev-integration DOMAIN="$DOMAIN"

echo "✓ All ASan integration tests passed."
