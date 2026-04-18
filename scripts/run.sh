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

# Default architecture
ARCH="arm"
ARCH_EXPLICIT=false

# Pre-scan arguments to find explicit --arch before processing input files
TEMP_ARGS=("$@")
while [[ $# -gt 0 ]]; do
  case $1 in
    --arch)
      ARCH="$2"
      ARCH_EXPLICIT=true
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done
set -- "${TEMP_ARGS[@]}"

# Process the input hardware description
DTB=""
IS_TEMP_DTB=false
EXTRA_ARGS=()
KERNEL=""
MACHINE_PROVIDED=false

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
      MACHINE_PROVIDED=true
      shift 2
      ;;
    --arch)
      # Handled above but consume it here
      shift 2
      ;;
    *)
      EXTRA_ARGS+=("$1")
      shift
      ;;
  esac
done

if [[ "$INPUT_FILE" == *.repl ]]; then
    echo "Processing Renode platform: $INPUT_FILE"
    DTB=$(mktemp /tmp/virtmcu-XXXXXX.dtb)
    ARCH_FILE=$(mktemp /tmp/virtmcu-XXXXXX.arch)
    IS_TEMP_DTB=true
    python3 -m tools.repl2qemu "$INPUT_FILE" --out-dtb "$DTB" --out-arch "$ARCH_FILE"
    if [ -f "$ARCH_FILE" ]; then
        ARCH=$(cat "$ARCH_FILE")
        rm "$ARCH_FILE"
    fi
elif [[ "$INPUT_FILE" == *.yaml ]]; then
    echo "Processing virtmcu YAML platform: $INPUT_FILE"
    DTB=$(mktemp /tmp/virtmcu-XXXXXX.dtb)
    CLI_FILE=$(mktemp /tmp/virtmcu-XXXXXX.cli)
    ARCH_FILE=$(mktemp /tmp/virtmcu-XXXXXX.arch)
    IS_TEMP_DTB=true
    python3 -m tools.yaml2qemu "$INPUT_FILE" --out-dtb "$DTB" --out-cli "$CLI_FILE" --out-arch "$ARCH_FILE"
    if [ -f "$ARCH_FILE" ]; then
        ARCH=$(cat "$ARCH_FILE")
        rm "$ARCH_FILE"
    fi
    if [ -f "$CLI_FILE" ]; then
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
    # Detect architecture from DTS (only if not explicitly overridden via --arch)
    if [ "$ARCH_EXPLICIT" = false ] && grep -iq "riscv" "$INPUT_FILE"; then
        ARCH="riscv"
    fi
elif [[ "$INPUT_FILE" == *.dtb ]]; then
    DTB="$INPUT_FILE"
fi

# Determine QEMU binary based on architecture
QEMU_ARCH_NAME="arm"
if [ "$ARCH" = "riscv" ] || [ "$ARCH" = "riscv64" ]; then
    QEMU_ARCH_NAME="riscv64"
elif [ "$ARCH" = "riscv32" ]; then
    QEMU_ARCH_NAME="riscv32"
fi

# Prioritize the build directory for developers
if [ -f "$QEMU_DIR/build-virtmcu/install/bin/qemu-system-$QEMU_ARCH_NAME" ]; then
    QEMU_BIN="$QEMU_DIR/build-virtmcu/install/bin/qemu-system-$QEMU_ARCH_NAME"
elif [ -f "$QEMU_DIR/build-virtmcu/qemu-system-$QEMU_ARCH_NAME" ]; then
    QEMU_BIN="$QEMU_DIR/build-virtmcu/qemu-system-$QEMU_ARCH_NAME"
    chmod +x "$QEMU_BIN"
else
    QEMU_BIN=$(command -v "qemu-system-$QEMU_ARCH_NAME" || echo "/opt/virtmcu/bin/qemu-system-$QEMU_ARCH_NAME")
fi

# Ensure QEMU has been built
if [ ! -f "$QEMU_BIN" ]; then
    echo "QEMU binary for $ARCH not found at $QEMU_BIN. Please run setup-qemu.sh first."
    exit 1
fi

# Default machine names
if [ "$MACHINE_PROVIDED" = false ]; then
    if [ "$ARCH" = "arm" ]; then
        MACHINE="arm-generic-fdt"
    elif [[ "$ARCH" == riscv* ]]; then
        MACHINE="virt"
        # Check if -bios is already in EXTRA_ARGS
        if [[ ! " ${EXTRA_ARGS[*]} " =~ " -bios " ]]; then
            EXTRA_ARGS+=("-bios" "none")
        fi
    fi
fi

# Set the QEMU module directory. 
# Prioritize the build directory for developers, fallback to installed location.
FOUND_SO=$(find "$QEMU_DIR/build-virtmcu/install" -name "hw-virtmcu-*.so" -type f 2>/dev/null | head -n1)
if [ -n "$FOUND_SO" ]; then
    QEMU_MODULE_DIR=$(dirname "$FOUND_SO")
elif [ -d "$QEMU_DIR/build-virtmcu/install/lib/qemu" ]; then
    QEMU_MODULE_DIR="$QEMU_DIR/build-virtmcu/install/lib/qemu"
elif [ -d "/opt/virtmcu/lib/aarch64-linux-gnu/qemu" ]; then
    QEMU_MODULE_DIR="/opt/virtmcu/lib/aarch64-linux-gnu/qemu"
elif [ -d "/opt/virtmcu/lib/x86_64-linux-gnu/qemu" ]; then
    QEMU_MODULE_DIR="/opt/virtmcu/lib/x86_64-linux-gnu/qemu"
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

# Docker build path
if [ -d "/build/zenoh-c/lib" ]; then
    export LD_LIBRARY_PATH="/build/zenoh-c/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
fi

# Installed virtmcu path
if [ -d "/opt/virtmcu/lib" ]; then
    export LD_LIBRARY_PATH="/opt/virtmcu/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
fi

# If a DTB is provided, handle it based on machine
if [ -n "$DTB" ]; then
    if [ "$MACHINE" = "arm-generic-fdt" ]; then
        MACHINE="${MACHINE},hw-dtb=${DTB}"
    else
        EXTRA_ARGS+=("-dtb" "$DTB")
    fi
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
    # Cleanup trap fires on EXIT
    trap 'rm -f "$DTB"' EXIT
    
    # Run QEMU in background so bash can handle signals immediately
    "${CMD[@]}" &
    QEMU_PID=$!
    
    # Traps for termination signals
    trap 'kill -TERM $QEMU_PID 2>/dev/null; wait $QEMU_PID; rm -f "$DTB"; exit 130' INT
    trap 'kill -TERM $QEMU_PID 2>/dev/null; wait $QEMU_PID; rm -f "$DTB"; exit 143' TERM
    
    wait $QEMU_PID
    exit $?
else
    # Direct execution replaces the shell process
    exec "${CMD[@]}"
fi
