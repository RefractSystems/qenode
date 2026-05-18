/*
 * tests/fixtures/guest_apps/bql_stress/main.c
 *
 * Bare-metal firmware to test BQL yielding in virtmcu.
 * This firmware enters a tight polling loop on the dummy peripheral.
 * If the emulator doesn't yield the BQL, the main loop will starve and the test
 * will timeout.
 */

#include <stdint.h>

#define UART0_BASE 0x09000000
#define DUMMY_BASE 0x09005000

// Using the reference-peripheral peripheral layout
#define REG_DUMMY_STATUS (DUMMY_BASE + 0x00)
#define STATUS_DATA_READY 0x01

void uart_putc(char c) { *(volatile uint32_t *)UART0_BASE = c; }

void uart_puts(const char *s) {
  while (*s) {
    uart_putc(*s++);
  }
}

void delay(int count) {
  for (int i = 0; i < count; i++) {
    asm volatile("nop");
  }
}

int main() {
  uart_puts("BQL stress starting\n");
  delay(10000);
  uart_puts("Tight polling loop\n");

  // No WFI, no delay. Pure CPU spin.
  // If the emulator doesn't yield the lock, the host is starved.
  while ((*(volatile uint32_t *)REG_DUMMY_STATUS & STATUS_DATA_READY) == 0) {
    // Spin forever until the status changes
  }

  uart_puts("Starvation avoided\n");

  while (1) {
    // Halt
  }

  return 0;
}
