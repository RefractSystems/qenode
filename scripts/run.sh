#!/usr/bin/env bash
# ==============================================================================
# run.sh
#
# This is a wrapper script to launch the locally built QEMU emulator.
# It automatically handles multiple hardware description formats and sets up
# the environment (like QEMU_MODULE_DIR) for dynamic loading.
#
# Usage:
#   ./scripts/run.sh [--repl|--yaml|--dts|--dtb <path>] [--kernel <path>] [args]
#
# Arguments:
#   --repl    Path to a Renode .repl file (auto-translated to DTB).
#   --yaml    Path to a virtmcu .yaml file (auto-translated to DTB).
#   --dts     Path to a Device Tree Source file (auto-compiled to DTB).
#   --dtb     Path to a pre-compiled Device Tree Blob.
#   --kernel  Path to the ELF kernel/firmware to boot.
#   --machine Name of the machine to emulate (defaults to arm-generic-fdt).
#   Any other arguments are passed directly to qemu-system-arm.
# ==============================================================================

set -e

# Resolve paths
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(dirname "$SCRIPT_DIR")"
QEMU_DIR="$WORKSPACE_DIR/third_party/qemu"
QEMU_BIN=$(command -v qemu-system-arm || echo "$QEMU_DIR/build-virtmcu/install/bin/qemu-system-arm")

# Set the QEMU module directory to point to our local build's lib/qemu (or multiarch equivalent)
# This is crucial for dynamic loading of our custom .so peripherals
# We explicitly search for .so files to avoid picking up stale .dylib files on macOS cross-builds
FOUND_SO=$(find "$QEMU_DIR/build-virtmcu/install" -name "hw-virtmcu-*.so" -type f 2>/dev/null | head -n1)
if [ -n "$FOUND_SO" ]; then
    QEMU_MODULE_DIR=$(dirname "$FOUND_SO")
elif [ -d "/opt/virtmcu/lib/qemu" ]; then
    QEMU_MODULE_DIR="/opt/virtmcu/lib/qemu"
else
    QEMU_MODULE_DIR="$QEMU_DIR/build-virtmcu/install/lib/qemu"
fi

# Add zenoh-c to LD_LIBRARY_PATH so QEMU can load the native Zenoh plugins
if [ -d "$WORKSPACE_DIR/third_party/zenoh-c/lib" ]; then
    export LD_LIBRARY_PATH="$WORKSPACE_DIR/third_party/zenoh-c/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
elif [ -d "$WORKSPACE_DIR/third_party/zenoh-c" ]; then
    export LD_LIBRARY_PATH="$WORKSPACE_DIR/third_party/zenoh-c${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
fi

if [ -d "/build/zenoh-c/lib" ]; then
    export LD_LIBRARY_PATH="/build/zenoh-c/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
fi

if [ -d "/opt/virtmcu/lib" ]; then
    export LD_LIBRARY_PATH="/opt/virtmcu/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
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
IS_TEMP_DTB=false

if [[ "$INPUT_FILE" == *.repl ]]; then
    echo "Processing Renode platform: $INPUT_FILE"
    DTB=$(mktemp /tmp/virtmcu-XXXXXX.dtb)
    IS_TEMP_DTB=true
    # Call our Phase 3 translator as a module
    python3 -m tools.repl2qemu "$INPUT_FILE" --out-dtb "$DTB"
elif [[ "$INPUT_FILE" == *.yaml ]]; then
    echo "Processing virtmcu YAML platform: $INPUT_FILE"
    DTB=$(mktemp /tmp/virtmcu-XXXXXX.dtb)
    CLI_FILE=$(mktemp /tmp/virtmcu-XXXXXX.cli)
    IS_TEMP_DTB=true
    # Call our Phase 3.5 translator as a module
    python3 -m tools.yaml2qemu "$INPUT_FILE" --out-dtb "$DTB" --out-cli "$CLI_FILE"
    if [ -f "$CLI_FILE" ]; then
        # Read the file line by line into the EXTRA_ARGS array
        while IFS= read -r line; do
            if [ -n "$line" ]; then
                EXTRA_ARGS+=("$line")
            fi
        done < "$CLI_FILE"
        rm "$CLI_FILE"
    fi
elif [[ "$INPUT_FILE" == *.dts ]]; then
    echo "Compiling Device Tree Source: $INPUT_FILE"
    DTB=$(mktemp /tmp/virtmcu-XXXXXX.dtb)
    IS_TEMP_DTB=true
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

echo "Running: ${CMD[*]}"

# If we have a temporary DTB, we must run QEMU as a child process and trap
# signals to ensure the file is cleaned up.
# If we have a permanent DTB, we use 'exec' to replace the shell process,
# which ensures correct PID tracking and signal propagation for callers.
if [ "$IS_TEMP_DTB" = true ]; then
    # Cleanup trap fires on EXIT, INT, and TERM
    trap 'rm -f "$DTB"' EXIT
    trap 'rm -f "$DTB"; exit 130' INT
    trap 'rm -f "$DTB"; exit 143' TERM
    
    # Run QEMU as a child
    "${CMD[@]}"
    exit $?
else
    # Direct execution replaces the shell process
    exec "${CMD[@]}"
fi
