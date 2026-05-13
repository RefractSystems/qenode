all: main.elf

main.elf: startup.S main.c
	arm-none-eabi-gcc -mcpu=cortex-a15 -marm -g -nostdlib -ffreestanding -T linker.ld startup.S main.c -o main.elf
