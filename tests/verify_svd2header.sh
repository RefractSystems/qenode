#!/bin/bash
set -ex

# Build virtmcu-cli first
cargo build -p virtmcu-cli

# Define paths
CLI="target/debug/virtmcu-cli"
SVD_FILE="hw/defs/actuator.svd"
OUT_HEADER="target/tmp/actuator_generated.h"
OUT_C="target/tmp/actuator_test.c"

# Create temp dir
mkdir -p target/tmp

# Generate header
$CLI platform svd2header $SVD_FILE -o $OUT_HEADER

# Create a minimal C file that includes the header
cat << 'EOF' > $OUT_C
#include "actuator_generated.h"

void test_func() {
    // Just a dummy function to ensure the file is not empty
    volatile uint32_t val = ACTUATOR->ID;
    (void)val;
}
EOF

# Compile the C file to verify _Static_asserts pass
arm-none-eabi-gcc -c -std=c11 -Wall -Werror $OUT_C -o target/tmp/actuator_test.o

echo "Test passed! SVD generation and Static Asserts are correct."
