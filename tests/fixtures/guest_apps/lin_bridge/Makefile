all: lin_echo.elf lin_master.elf lin_slave.elf

%.elf: %.S
	arm-none-eabi-gcc -mcpu=cortex-a15 -nostdlib -g -T lin_echo.ld $< -o $@

clean:
	rm -f *.elf
