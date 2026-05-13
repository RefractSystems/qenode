CC = arm-none-eabi-gcc
CFLAGS = -mcpu=cortex-a15 -mfpu=vfpv4 -mfloat-abi=softfp -nostdlib -g -O2 -I. -Igenerated
LDFLAGS = -T linker.ld -lgcc

# VirtMCU CLI command (passed from environment or default)
VIRTMCU_CLI ?= cargo run -p virtmcu-cli --

all: generated/actuator.h generated/sensor.h controller.elf

generated/actuator.h: ../../../../hw/defs/actuator.svd
	mkdir -p generated
	$(VIRTMCU_CLI) platform svd2-header $< --output $@

generated/sensor.h: ../../../../hw/defs/sensor.svd
	mkdir -p generated
	$(VIRTMCU_CLI) platform svd2-header $< --output $@

controller.elf: startup.S controller.c generated/actuator.h generated/sensor.h
	$(CC) $(CFLAGS) startup.S controller.c $(LDFLAGS) -o controller.elf

clean:
	rm -f controller.elf
	rm -rf generated
