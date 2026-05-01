# ==============================================================================
# Makefile (Lesson 16: Performance)
# ==============================================================================

CROSS_COMPILE ?= arm-none-eabi-
CC = $(CROSS_COMPILE)gcc
CFLAGS = -mcpu=cortex-a15 -nostdlib -O2 -g

all: bench.elf minimal.dtb

bench.elf: src/startup.o src/bench.o src/bench.ld
	$(CC) $(CFLAGS) -T src/bench.ld src/startup.o src/bench.o -o bench.elf

src/startup.o: src/startup.S
	$(CC) $(CFLAGS) -c src/startup.S -o src/startup.o

src/bench.o: src/bench.c
	$(CC) $(CFLAGS) -c src/bench.c -o src/bench.o

minimal.dtb: src/minimal.dts
	dtc -I dts -O dtb -o minimal.dtb src/minimal.dts

clean:
	rm -f src/*.o bench.elf minimal.dtb

.PHONY: all clean
