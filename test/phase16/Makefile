CROSS_COMPILE ?= arm-none-eabi-
CC = $(CROSS_COMPILE)gcc
CFLAGS = -mcpu=cortex-a15 -nostdlib -O2 -g

all: bench.elf

bench.elf: startup.o bench.o bench.ld
	$(CC) $(CFLAGS) -T bench.ld startup.o bench.o -o bench.elf

startup.o: startup.S
	$(CC) $(CFLAGS) -c startup.S -o startup.o

bench.o: bench.c
	$(CC) $(CFLAGS) -c bench.c -o bench.o

clean:
	rm -f *.o bench.elf

.PHONY: all clean
