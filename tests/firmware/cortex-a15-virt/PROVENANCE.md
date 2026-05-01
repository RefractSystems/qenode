# cortex-a15-virt — Firmware Provenance

| Field        | Value |
|---|---|
| Status       | `virtmcu-baseline` — silicon validation pending |
| ELF source   | `tests/fixtures/guest_apps/uart_echo/echo.S` |
| Build command | `arm-none-eabi-gcc -mcpu=cortex-a15 -nostdlib -T echo.ld echo.S -o echo.elf` |
| Target CPU   | ARM Cortex-A15, soft-float ABI |
| Entry point  | `0x40000000` |
| UART         | PL011 at `0x09000000` (matches `tests/fixtures/guest_apps/boot_arm/minimal.dtb`) |
| SHA256       | `810886b91efbc2c9c6ff71ffe3b500eba261e35e788926488e9cb176a24a18ec` |

## Golden output provenance

`golden_uart.txt` was derived from reading the firmware source and is confirmed to match
VirtMCU output by `tests/test_interactive_echo.robot`. It has not been captured from
real Cortex-A15 silicon.

## To silicon-validate

1. Obtain a board with a PL011 UART mapped at `0x09000000` and RAM at `0x40000000`.
2. Flash `echo.elf` (raw binary load, not packaged) via JTAG or bootloader.
3. Connect a 3.3V serial adapter at the board's UART pins (115200 8N1 by default for PL011).
4. Capture boot output to `golden_uart.txt` (strip terminal escape codes if any).
5. Run `sha256sum echo.elf` and confirm it matches the SHA256 in this file.
6. Update `SHA256SUMS` provenance comment and this file's Status field.
7. Commit with: `test(binary-fidelity): silicon-validate cortex-a15-virt echo on <board> <date>`.
