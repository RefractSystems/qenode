# tests/fixtures/guest_apps//Makefile
CROSS_COMPILE ?= arm-none-eabi-
CC = $(CROSS_COMPILE)gcc
AS = $(CROSS_COMPILE)as
LD = $(CROSS_COMPILE)ld
OBJCOPY = $(CROSS_COMPILE)objcopy

CFLAGS = -mcpu=cortex-a15 -marm -g -nostdlib -ffreestanding
LDFLAGS = -T linker.ld

all: radio_test.elf

radio_test.elf: startup.S radio_test.c
	$(CC) $(CFLAGS) $(LDFLAGS) startup.S radio_test.c -o radio_test.elf

clean:
	rm -f radio_test.elf
