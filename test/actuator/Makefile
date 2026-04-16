# test/actuator/Makefile
CROSS_COMPILE ?= arm-none-eabi-
CC = $(CROSS_COMPILE)gcc
AS = $(CROSS_COMPILE)as
LD = $(CROSS_COMPILE)ld
OBJCOPY = $(CROSS_COMPILE)objcopy

CFLAGS = -mcpu=cortex-a15 -marm -g -nostdlib -ffreestanding
LDFLAGS = -T linker.ld

all: actuator.elf

actuator.elf: startup.S actuator.c
	$(CC) $(CFLAGS) $(LDFLAGS) startup.S actuator.c -o actuator.elf

clean:
	rm -f actuator.elf
