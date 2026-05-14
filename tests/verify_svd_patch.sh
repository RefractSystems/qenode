#!/bin/bash
set -e

INPUT_SVD="hw/defs/actuator.svd"
PATCH_YAML="tests/fixtures/svd_patching/patch.yaml"
OUTPUT_SVD="tests/fixtures/svd_patching/patched_actuator.svd"

echo "Applying patch..."
cargo +nightly run -Z bindeps -p virtmcu-cli -- platform patch-svd "$INPUT_SVD" "$PATCH_YAML" --output "$OUTPUT_SVD"

echo "Checking patched SVD..."
# We expect the GO register to have a resetValue of 0x1. 
# Depending on how the agent implemented it, it might add a <resetValue> tag.
grep -A 5 "<name>GO</name>" "$OUTPUT_SVD" | grep -q "0x1"

if [ $? -eq 0 ]; then
    echo "SUCCESS: resetValue was successfully patched."
    rm "$OUTPUT_SVD"
    exit 0
else
    echo "ERROR: resetValue was not found in the patched SVD."
    cat "$OUTPUT_SVD"
    rm "$OUTPUT_SVD"
    exit 1
fi
