CC = arm-none-eabi-gcc
CFLAGS = -mcpu=cortex-a9 -nostdlib -g
LDFLAGS = -T linker.ld

all: test_wfi.elf test_mmio.elf

test_wfi.elf: test_wfi.S linker.ld
	$(CC) $(CFLAGS) $(LDFLAGS) test_wfi.S -o test_wfi.elf

test_mmio.elf: test_mmio.S linker.ld
	$(CC) $(CFLAGS) $(LDFLAGS) test_mmio.S -o test_mmio.elf

clean:
	rm -f *.elf *.dtb *.cli *.arch *.log
