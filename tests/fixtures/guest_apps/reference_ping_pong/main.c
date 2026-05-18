/*
 * tests/fixtures/guest_apps/reference_ping_pong/main.c
 *
 * Bare-metal firmware to test the multi-node reference-peripheral network ping-pong.
 */

#include <stdint.h>
#include "reference_regs.h"

#define UART0_BASE 0x09000000
#define STATUS_DATA_READY 0x01

#ifndef NODE_ID
#error "NODE_ID must be defined"
#endif

void uart_putc(char c) { *(volatile uint32_t *)UART0_BASE = c; }

void uart_puts(const char *s) {
  while (*s) {
    uart_putc(*s++);
  }
}

int main() {
  if (NODE_ID == 0) {
    uart_puts("N0:start\n");

    // Send Ping
    REFERENCE_PERIPHERAL->TX = 0x50494E47; // "PING"
    uart_puts("N0:ping\n");

    // Wait for Pong (Tests Yield-on-Read / BQL safety)
    while ((REFERENCE_PERIPHERAL->STATUS & STATUS_DATA_READY) == 0) {
    }

    uart_puts("N0:pong rx\n");

  } else if (NODE_ID == 1) {
    uart_puts("N1:start\n");

    // Wait for Ping (Tests Yield-on-Read / BQL safety)
    while ((REFERENCE_PERIPHERAL->STATUS & STATUS_DATA_READY) == 0) {
    }
    uart_puts("N1:ping rx\n");

    // Send Pong
    REFERENCE_PERIPHERAL->TX = 0x504F4E47; // "PONG"
    uart_puts("N1:pong\n");
  }

  while (1) {
    // Halt
  }

  return 0;
}
