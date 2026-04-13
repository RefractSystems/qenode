CROSS_COMPILE ?= riscv64-linux-gnu-
AS = $(CROSS_COMPILE)as
LD = $(CROSS_COMPILE)ld
OBJCOPY = $(CROSS_COMPILE)objcopy
DTC = dtc

ASFLAGS = -march=rv64imafdc -mabi=lp64d
LDFLAGS = -T linker.ld

all: hello.elf minimal.dtb

hello.elf: hello.S linker.ld
	$(AS) $(ASFLAGS) hello.S -o hello.o
	$(LD) $(LDFLAGS) hello.o -o hello.elf

minimal.dtb: minimal.dts
	$(DTC) -I dts -O dtb -o minimal.dtb minimal.dts

clean:
	rm -f *.o *.elf *.dtb
