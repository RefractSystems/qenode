CC = arm-none-eabi-gcc
AS = arm-none-eabi-as
LD = arm-none-eabi-ld
OBJCOPY = arm-none-eabi-objcopy
CFLAGS = -mcpu=cortex-a15 -nostdlib -g

all: firmware.elf

firmware.elf: firmware.S linker.ld
	$(CC) $(CFLAGS) -T linker.ld firmware.S -o firmware.elf

clean:
	rm -f firmware.elf
