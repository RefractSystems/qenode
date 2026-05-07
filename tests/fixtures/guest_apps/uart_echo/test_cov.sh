#!/usr/bin/env bash

set -euo pipefail
QEMU="${QEMU_BIN:-$(which qemu-system-arm 2>/dev/null || echo "qemu-system-arm")}"

echo -e "Test\x1b" | "$QEMU" -M arm-generic-fdt,hw-dtb=tests/fixtures/guest_apps/boot_arm/minimal.dtb \
    -kernel tests/fixtures/guest_apps/uart_echo/echo.elf -nographic -m 128M -display none \
    -semihosting -semihosting-config enable=on,target=native \
    -serial stdio
