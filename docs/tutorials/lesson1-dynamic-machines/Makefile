# ==============================================================================
# Makefile (Lesson 1: Dynamic Machines)
#
# This Makefile automates the building of the lesson's artifacts:
#   1. minimal.dtb: The compiled Device Tree Blob generated from the source (.dts).
#   2. hello.elf: The minimal bare-metal ARM assembly kernel compiled from hello.S.
# ==============================================================================

# Toolchain setup for cross-compiling to ARM
CC = arm-none-eabi-gcc
# Target Cortex-A15, skip standard libraries (bare-metal), and include debug symbols
CFLAGS = -mcpu=cortex-a15 -nostdlib -g
# Use the provided linker script to set the correct memory layout
LDFLAGS = -T src/linker.ld

# Default target builds both the device tree and the kernel executable
all: minimal.dtb hello.elf

# Compile the device tree source into a blob using the Device Tree Compiler (dtc)
minimal.dtb: src/minimal.dts
	dtc -I dts -O dtb -o minimal.dtb src/minimal.dts

# Cross-compile the assembly source into the final ELF executable
hello.elf: src/hello.S src/linker.ld
	$(CC) $(CFLAGS) $(LDFLAGS) src/hello.S -o hello.elf

# Target to clean up generated files
clean:
	rm -f *.dtb *.elf
