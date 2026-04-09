#!/usr/bin/env bash
# ==============================================================================
# smoke_test.sh
#
# This script performs a basic "smoke test" for Phase 1 of the qenode project.
# It uses the `run.sh` script to boot the `hello.elf` kernel on the 
# `arm-generic-fdt` machine described by `minimal.dtb`. 
#
# The `hello.elf` kernel simply writes the string "HI\n" to the PL011 UART.
# This script captures the UART output to a file and verifies that the string 
# "HI" is present, ensuring that:
#   1. The QEMU binary works.
#   2. The FDT is parsed and the machine boots.
#   3. The CPU successfully executes the test kernel.
#   4. The UART is correctly instantiated and mapped.
# ==============================================================================

set -e

# Resolve paths
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(dirname "$(dirname "$SCRIPT_DIR")")"
RUN_SH="$WORKSPACE_DIR/scripts/run.sh"

DTB="$SCRIPT_DIR/minimal.dtb"
KERNEL="$SCRIPT_DIR/hello.elf"

# Verify required files exist before running the test
if [ ! -f "$DTB" ]; then
    echo "DTB not found: $DTB"
    exit 1
fi

if [ ! -f "$KERNEL" ]; then
    echo "Kernel not found: $KERNEL"
    exit 1
fi

echo "Running smoke test..."
OUTPUT_LOG="smoke_test_output.log"
# Ensure we start with a clean log
rm -f "$OUTPUT_LOG"

# Run QEMU via the wrapper script with a 3-second timeout to prevent hangs.
#
# Flag rationale:
#   -serial file:$OUTPUT_LOG   Explicit serial device.  Passing -serial sets
#                               default_serial=0 in vl.c, so -nographic does NOT
#                               also add "stdio" as serial 0.  The file becomes
#                               serial 0 and the PL011 (chardev=<0x00>) writes to it.
#   -nographic                  Suppresses the SDL/GTK display window.
#   -monitor none               Suppresses the QEMU monitor (which -nographic would
#                               otherwise redirect to stdio, cluttering test output).
#
# We append `|| true` so the script doesn't abort when `timeout` kills the QEMU process.
timeout 3s "$RUN_SH" --dtb "$DTB" --kernel "$KERNEL" -nographic -monitor none -serial file:"$OUTPUT_LOG" || true

# Check if the output log contains the expected "HI" string from the kernel
if [ -f "$OUTPUT_LOG" ] && grep -q "HI" "$OUTPUT_LOG"; then
    echo "PASSED: UART output 'HI' detected"
    rm "$OUTPUT_LOG"
    exit 0
else
    echo "FAILED: UART output 'HI' not found"
    if [ -f "$OUTPUT_LOG" ]; then
        echo "Full output:"
        cat "$OUTPUT_LOG"
    else
        echo "No output file created."
    fi
    exit 1
fi
