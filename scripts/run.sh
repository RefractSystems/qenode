#!/usr/bin/env bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(dirname "$SCRIPT_DIR")"
QEMU_DIR="$WORKSPACE_DIR/third_party/qemu"
QEMU_BIN="$QEMU_DIR/build-qenode/qemu-system-arm"
QEMU_MODULE_DIR="$QEMU_DIR/build-qenode"

if [ ! -f "$QEMU_BIN" ]; then
    echo "QEMU binary not found at $QEMU_BIN. Please run setup-qemu.sh first."
    exit 1
fi

DTB=""
KERNEL=""
MACHINE="arm-generic-fdt"
EXTRA_ARGS=()

while [[ $# -gt 0 ]]; do
  case $1 in
    --dtb)
      DTB="$2"
      shift 2
      ;;
    --kernel)
      KERNEL="$2"
      shift 2
      ;;
    --machine)
      MACHINE="$2"
      shift 2
      ;;
    *)
      EXTRA_ARGS+=("$1")
      shift
      ;;
  esac
done

if [ -n "$DTB" ]; then
    MACHINE="${MACHINE},hw-dtb=${DTB}"
fi

CMD=("$QEMU_BIN" "-M" "$MACHINE")

if [ -n "$KERNEL" ]; then
    CMD+=("-kernel" "$KERNEL")
fi

CMD+=("${EXTRA_ARGS[@]}")

export QEMU_MODULE_DIR
echo "Running: ${CMD[@]}"
exec "${CMD[@]}"
