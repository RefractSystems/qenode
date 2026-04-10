#!/usr/bin/env bash
# ==============================================================================
# run.sh
#
# This is a wrapper script to launch the locally built QEMU emulator.
# It automatically sets up the environment (like QEMU_MODULE_DIR) so QEMU
# can find our custom QOM plugins (.so files) without needing global installation.
#
# Usage:
#   ./scripts/run.sh [--dtb <path/to/dtb>] [--kernel <path/to/elf>] [other qemu args]
#
# Arguments:
#   --dtb     Path to the Device Tree Blob (DTB) file. Appends to the machine string.
#   --kernel  Path to the ELF kernel/firmware to boot.
#   --machine Name of the machine to emulate (defaults to arm-generic-fdt).
#   Any other arguments are passed directly to qemu-system-arm.
# ==============================================================================

set -e

# Resolve paths
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(dirname "$SCRIPT_DIR")"
QEMU_DIR="$WORKSPACE_DIR/third_party/qemu"
QEMU_BIN="$QEMU_DIR/build-qenode/install/bin/qemu-system-arm"

# Set the QEMU module directory to point to our local build's lib/qemu (or multiarch equivalent)
# This is crucial for dynamic loading of our custom .so peripherals
# We explicitly search for .so files to avoid picking up stale .dylib files on macOS cross-builds
FOUND_SO=$(find "$QEMU_DIR/build-qenode/install" -name "hw-qenode-*.so" -type f 2>/dev/null | head -n1)
if [ -n "$FOUND_SO" ]; then
    QEMU_MODULE_DIR=$(dirname "$FOUND_SO")
else
    QEMU_MODULE_DIR="$QEMU_DIR/build-qenode/install/lib/qemu"
fi

# Ensure QEMU has been built
if [ ! -f "$QEMU_BIN" ]; then
    echo "QEMU binary not found at $QEMU_BIN. Please run setup-qemu.sh first."
    exit 1
fi

# Parse arguments
INPUT_FILE=""
KERNEL=""
MACHINE="arm-generic-fdt"
EXTRA_ARGS=()

while [[ $# -gt 0 ]]; do
  case $1 in
    --repl|--yaml)
      INPUT_FILE="$2"
      shift 2
      ;;
    --dtb|--dts)
      INPUT_FILE="$2"
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
      EXTRA_ARGS+=("$1") # Collect any remaining arguments to pass to QEMU
      shift
      ;;
  esac
done

# Process the input hardware description
DTB=""
if [[ "$INPUT_FILE" == *.repl ]]; then
    echo "Processing Renode platform: $INPUT_FILE"
    DTB="${INPUT_FILE%.repl}.dtb"
    # Call our Phase 3 translator
    python3 "$WORKSPACE_DIR/tools/repl2qemu/__main__.py" "$INPUT_FILE" --out-dtb "$DTB"
elif [[ "$INPUT_FILE" == *.yaml ]]; then
    echo "Processing qenode YAML platform: $INPUT_FILE"
    DTB="${INPUT_FILE%.yaml}.dtb"
    # Call our Phase 3.5 translator
    python3 "$WORKSPACE_DIR/tools/yaml2qemu.py" "$INPUT_FILE" --out-dtb "$DTB"
elif [[ "$INPUT_FILE" == *.dts ]]; then
    echo "Compiling Device Tree Source: $INPUT_FILE"
    DTB="${INPUT_FILE%.dts}.dtb"
    dtc -I dts -O dtb -o "$DTB" "$INPUT_FILE"
elif [[ "$INPUT_FILE" == *.dtb ]]; then
    DTB="$INPUT_FILE"
fi

# If a DTB is provided (either directly or generated), append it to the machine parameter
if [ -n "$DTB" ]; then
    MACHINE="${MACHINE},hw-dtb=${DTB}"
fi

# Build the command array
CMD=("$QEMU_BIN" "-M" "$MACHINE")

if [ -n "$KERNEL" ]; then
    CMD+=("-kernel" "$KERNEL")
fi

CMD+=("${EXTRA_ARGS[@]}")

# Export QEMU_MODULE_DIR so the QEMU binary picks it up
export QEMU_MODULE_DIR

echo "Running: ${CMD[@]}"
# Replace the shell process with the QEMU process
exec "${CMD[@]}"
