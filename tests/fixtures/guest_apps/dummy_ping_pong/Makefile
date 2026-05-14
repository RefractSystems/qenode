# Makefile for dummy_ping_pong

CC = arm-none-eabi-gcc
OBJCOPY = arm-none-eabi-objcopy
CFLAGS = -mcpu=cortex-a15 -nostdlib -nostartfiles -ffreestanding

all: pinger.bin ponger.bin

pinger.elf: main.c startup.S ../boot_arm/linker.ld
	$(CC) $(CFLAGS) -DNODE_ID=0 -T ../boot_arm/linker.ld startup.S main.c -o pinger.elf

ponger.elf: main.c startup.S ../boot_arm/linker.ld
	$(CC) $(CFLAGS) -DNODE_ID=1 -T ../boot_arm/linker.ld startup.S main.c -o ponger.elf

pinger.bin: pinger.elf
	$(OBJCOPY) -O binary pinger.elf pinger.bin

ponger.bin: ponger.elf
	$(OBJCOPY) -O binary ponger.elf ponger.bin

clean:
	rm -f *.elf *.bin
