# ==============================================================================
# Makefile (Phase 1)
#
# This Makefile builds the necessary artifacts for the Phase 1 smoke test.
# It compiles the Device Tree Source (.dts) into a binary Device Tree Blob (.dtb)
# using the Device Tree Compiler (dtc). It also cross-compiles a minimal ARM
# bare-metal assembly kernel (hello.S) into an ELF executable using a custom
# linker script (linker.ld).
# ==============================================================================

# Toolchain configuration
CC = arm-none-eabi-gcc
# Target the Cortex-A15, do not use standard C libraries, and include debug symbols
CFLAGS = -mcpu=cortex-a15 -nostdlib -g
# Use the local linker script to place the code at the correct memory address (0x40000000)
LDFLAGS = -T linker.ld

# Default target builds both the DTB and the ELF kernel
all: minimal.dtb hello.elf

# Compile the device tree source into a blob
minimal.dtb: minimal.dts
	dtc -I dts -O dtb -o minimal.dtb minimal.dts

# Cross-compile the assembly source into an ELF executable
hello.elf: hello.S linker.ld
	$(CC) $(CFLAGS) $(LDFLAGS) hello.S -o hello.elf

# Clean up generated artifacts
clean:
	rm -f *.dtb *.elf
