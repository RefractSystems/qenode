#!/usr/bin/env bash

set -euo pipefail
echo -e "Test\x1b" | qemu-system-arm -M arm-generic-fdt,hw-dtb=test/phase1/minimal.dtb \
    -kernel test/phase8/echo.elf -nographic -m 128M -display none \
    -semihosting -semihosting-config enable=on,target=native \
    -serial stdio
