CC = arm-none-eabi-gcc
CFLAGS = -mcpu=cortex-a15 -nostdlib -g -O2
LDFLAGS = -T linker.ld -lgcc

all: controller.elf

controller.elf: startup.S controller.c
	arm-none-eabi-gcc -mcpu=cortex-a15 -nostdlib -g -O2 -T linker.ld startup.S controller.c -lgcc -o controller.elf

clean:
	rm -f controller.elf
