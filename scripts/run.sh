#!/usr/bin/env bash
# run.sh — Launch wrapper for qenode-patched QEMU.
#
# Sets the module path to the installed QEMU module dir so that
# `hw-qenode-*.so` plugins are discoverable via -device <name>.
#
# Usage:
#   ./scripts/run.sh -M arm-generic-fdt -hw-dtb board.dtb -kernel fw.elf [...]
#   ./scripts/run.sh --arch aarch64 -M arm-generic-fdt ...   (default: arm)
#
# Environment variables:
#   QEMU_SRC    QEMU source / build root (default: ext/qemu)
#   QEMU_BUILD  Build directory (default: $QEMU_SRC/build-qenode)
#   ARCH        arm or aarch64 (default: arm)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

QEMU_SRC="${QEMU_SRC:-$REPO_ROOT/ext/qemu}"
QEMU_BUILD="${QEMU_BUILD:-$QEMU_SRC/build-qenode}"
ARCH="${ARCH:-arm}"

QEMU_BIN="$QEMU_BUILD/install/bin/qemu-system-$ARCH"
MODULE_DIR="$QEMU_BUILD/install/lib/qemu"

if [ ! -x "$QEMU_BIN" ]; then
  echo "ERROR: QEMU binary not found at $QEMU_BIN"
  echo "       Run ./scripts/setup-qemu.sh first."
  exit 1
fi

# Override arch from --arch flag if present (consumed here, not passed to QEMU)
ARGS=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --arch)
      ARCH="$2"
      QEMU_BIN="$QEMU_BUILD/install/bin/qemu-system-$ARCH"
      shift 2
      ;;
    *)
      ARGS+=("$1")
      shift
      ;;
  esac
done

export QEMU_MODULE_DIR="$MODULE_DIR"

echo "==> qenode run"
echo "    binary    : $QEMU_BIN"
echo "    module dir: $MODULE_DIR"
echo ""

exec "$QEMU_BIN" "${ARGS[@]}"
