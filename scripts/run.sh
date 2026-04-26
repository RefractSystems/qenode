#!/usr/bin/env bash
# ==============================================================================
# run.sh
#
# This is a wrapper script to launch the locally built QEMU emulator.
# It automatically handles multiple hardware description formats and sets up
# the environment (like QEMU_MODULE_DIR) for dynamic plugin loading.
#
# PLUGIN STALENESS PREVENTION:
# This script is designed to recursively search the `$BUILD_DIR_NAME` directory
# to prioritize the freshest compiled `.so` artifacts over globally installed
# ones, ensuring developers and agents are always testing their latest code.
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

set -euo pipefail

# SOTA Error visibility
trap 'echo "ERROR: ${BASH_SOURCE[0]} failed at line $LINENO" >&2' ERR

# Resolve paths
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(dirname "$SCRIPT_DIR")"
QEMU_DIR="$WORKSPACE_DIR/third_party/qemu"

# Inherit optional env vars with safe defaults for -u compatibility
VIRTMCU_SKIP_BUILD_DIR="${VIRTMCU_SKIP_BUILD_DIR:-}"

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
INPUT_FILE=""

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

# Prioritize the build directory for developers, unless skipped
BUILD_DIR_NAME="build-virtmcu$( [ "${VIRTMCU_USE_ASAN:-0}" = "1" ] && echo "-asan" || echo "" )"

if [[ "${VIRTMCU_SKIP_BUILD_DIR:-0}" == "1" ]]; then
    # Strictly use installed path when skipping build dir
    QEMU_BIN="/opt/virtmcu/bin/qemu-system-$QEMU_ARCH_NAME"
    if [ ! -f "$QEMU_BIN" ]; then
        QEMU_BIN=$(command -v "qemu-system-$QEMU_ARCH_NAME" || true)
    fi
elif [ -f "$QEMU_DIR/$BUILD_DIR_NAME/install/bin/qemu-system-$QEMU_ARCH_NAME" ]; then
    QEMU_BIN="$QEMU_DIR/$BUILD_DIR_NAME/install/bin/qemu-system-$QEMU_ARCH_NAME"
elif [ -f "$QEMU_DIR/$BUILD_DIR_NAME/qemu-system-$QEMU_ARCH_NAME" ]; then
    QEMU_BIN="$QEMU_DIR/$BUILD_DIR_NAME/qemu-system-$QEMU_ARCH_NAME"
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

# ── Module Discovery ──────────────────────────────────────────────────────────

# Search for the module directory containing VirtMCU plugins.
# We prioritize the freshest build artifacts, then standard installation paths.
QEMU_MODULE_DIR=""

# 1. Check for a fresh .so in the build tree (recursive search)
if [[ "${VIRTMCU_SKIP_BUILD_DIR:-0}" != "1" ]] && [ -d "$QEMU_DIR/$BUILD_DIR_NAME" ]; then
    # Find freshest plugin, avoiding the 'qemu-bundle' artifacts
    FRESH_SO=$(find "$QEMU_DIR/$BUILD_DIR_NAME" -name "hw-virtmcu-*.so*" -not -path "*/qemu-bundle/*" \( -type f -o -type l \) -printf '%T@ %p\n' 2>/dev/null | sort -n | tail -1 | cut -f2- -d" " || true)
    if [ -n "$FRESH_SO" ]; then
        QEMU_MODULE_DIR=$(dirname "$FRESH_SO")
    fi
fi

# 2. If not found, check standard paths in priority order
if [ -z "$QEMU_MODULE_DIR" ]; then
    POSSIBLE_PATHS=()
    if [[ "${VIRTMCU_SKIP_BUILD_DIR:-0}" != "1" ]]; then
        POSSIBLE_PATHS+=(
            "$QEMU_DIR/$BUILD_DIR_NAME/install/lib/aarch64-linux-gnu/qemu"
            "$QEMU_DIR/$BUILD_DIR_NAME/install/lib/x86_64-linux-gnu/qemu"
            "$QEMU_DIR/$BUILD_DIR_NAME/install/lib/qemu"
        )
    fi
    POSSIBLE_PATHS+=(
        "/opt/virtmcu/lib/aarch64-linux-gnu/qemu"
        "/opt/virtmcu/lib/x86_64-linux-gnu/qemu"
        "/opt/virtmcu/lib/qemu"
    )

    for p in "${POSSIBLE_PATHS[@]}"; do
        if [ -d "$p" ] && find "$p" -name "hw-virtmcu-*.so*" -print -quit | grep -q .; then
            QEMU_MODULE_DIR="$p"
            break
        fi
    done
fi

# 3. Final fallback: use the build dir path even if missing (let QEMU fail with a better error)
if [ -z "$QEMU_MODULE_DIR" ] && [[ "${VIRTMCU_SKIP_BUILD_DIR:-0}" != "1" ]]; then
    QEMU_MODULE_DIR="$QEMU_DIR/$BUILD_DIR_NAME/install/lib/qemu"
    mkdir -p "$QEMU_MODULE_DIR"
fi

# ASan Instrumentation Check
has_asan() {
    local file="$1"
    if [ ! -f "$file" ]; then return 1; fi
    # Check for ASan initialization symbols which indicate instrumentation
    if strings "$file" 2>/dev/null | grep "__asan_init" >/dev/null; then
        return 0
    fi
    return 1
}

check_asan_mismatch() {
    local bin="$1"
    local mod_dir="$2"
    local bin_asan=false
    local mod_asan=false

    if has_asan "$bin"; then bin_asan=true; fi

    # Use find -L so versioned symlinks (e.g. .so -> .so.1) are followed correctly.
    local sample_plugin
    if [ -d "$mod_dir" ]; then sample_plugin=$(find -L "$mod_dir" -name "hw-virtmcu-*.so*" -type f -print -quit 2>/dev/null || true); fi

    # If no plugins are present yet, skip the mismatch check — the missing-plugin
    # warning at the end of this script is the correct diagnostic in that case.
    if [ -z "$sample_plugin" ]; then return 0; fi

    if has_asan "$sample_plugin"; then
        mod_asan=true
    fi

    if [ "$bin_asan" != "$mod_asan" ]; then
        echo "=============================================================================="
        echo "FATAL: ASan Instrumentation Mismatch Detected!"
        echo "------------------------------------------------------------------------------"
        echo "QEMU Binary ($bin): ASan=$( [ "$bin_asan" = true ] && echo "YES" || echo "NO" )"
        echo "QEMU Modules ($mod_dir): ASan=$( [ "$mod_asan" = true ] && echo "YES" || echo "NO" )"
        echo "------------------------------------------------------------------------------"
        echo "Mixing instrumented and non-instrumented code causes 'ASan runtime does not"
        echo "come first' errors or silent corruption."
        echo ""
        if [ "$bin_asan" = true ]; then
            echo "Action: Rebuild your plugins with VIRTMCU_USE_ASAN=1 or use a non-ASan QEMU."
        else
            echo "Action: Rebuild your plugins without ASan or use an ASan-instrumented QEMU."
        fi
        echo "=============================================================================="
        exit 1
    fi
}

# Perform pre-flight ASan check
check_asan_mismatch "$QEMU_BIN" "$QEMU_MODULE_DIR"

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

# Validate plugin discovery
if [ -n "$QEMU_MODULE_DIR" ]; then
    if [ ! -d "$QEMU_MODULE_DIR" ] || ! find "$QEMU_MODULE_DIR" -name "hw-virtmcu-*.so*" -print -quit | grep -q .; then
        echo "WARNING: QEMU_MODULE_DIR ($QEMU_MODULE_DIR) is missing or contains no virtmcu plugins! Simulation boot will likely fail." >&2
    fi
    export QEMU_MODULE_DIR
fi

# Automatically handle ASan LD_PRELOAD if QEMU is instrumented.
# AddressSanitizer requires that its runtime library be loaded first.
if ldd "$QEMU_BIN" | grep "libasan" >/dev/null; then
    LIBASAN=$(ldd "$QEMU_BIN" | grep "libasan" | awk '{print $3}' || true)
    if [ -n "$LIBASAN" ] && [ -f "$LIBASAN" ]; then
        export LD_PRELOAD="$LIBASAN${LD_PRELOAD:+:$LD_PRELOAD}"
    fi
fi

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
    trap 'kill -TERM $QEMU_PID 2>/dev/null || true; wait $QEMU_PID; rm -f "$DTB"; exit 130' INT
    trap 'kill -TERM $QEMU_PID 2>/dev/null || true; wait $QEMU_PID; rm -f "$DTB"; exit 143' TERM
    
    wait $QEMU_PID
    exit $?
else
    # Direct execution replaces the shell process
    exec "${CMD[@]}"
fi
