CC = arm-none-eabi-gcc
CFLAGS = -mcpu=cortex-a15 -nostdlib -g
LDFLAGS = -T src/linker.ld

all: minimal.dtb hello.elf

minimal.dtb: src/minimal.dts
	dtc -I dts -O dtb -o minimal.dtb src/minimal.dts

hello.elf: src/hello.S src/linker.ld
	$(CC) $(CFLAGS) $(LDFLAGS) src/hello.S -o hello.elf

clean:
	rm -f *.dtb *.elf
