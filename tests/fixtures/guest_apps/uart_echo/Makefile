CROSS_COMPILE ?= arm-none-eabi-

all: echo.elf

echo.elf: echo.S echo.ld
	$(CROSS_COMPILE)gcc -mcpu=cortex-a15 -nostdlib -T echo.ld echo.S -o echo.elf

clean:
	rm -f echo.elf

.PHONY: all clean
