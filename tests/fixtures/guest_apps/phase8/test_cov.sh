#!/usr/bin/env bash

set -euo pipefail
echo -e "Test\x1b" | qemu-system-arm -M arm-generic-fdt,hw-dtb=tests/fixtures/guest_apps/phase1/minimal.dtb \
    -kernel tests/fixtures/guest_apps/phase8/echo.elf -nographic -m 128M -display none \
    -semihosting -semihosting-config enable=on,target=native \
    -serial stdio
