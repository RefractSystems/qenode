/*
 * tests/fixtures/guest_apps/dummy_ping_pong/main.c
 *
 * Bare-metal firmware to test the multi-node rust-dummy network ping-pong.
 *
 * If compiled with -DNODE_ID=0 (The Pinger):
 *   1. Sends a payload to REG_DUMMY_TX.
 *   2. Spins on REG_DUMMY_STATUS waiting for the Pong.
 *   3. Exits cleanly.
 *
 * If compiled with -DNODE_ID=1 (The Ponger):
 *   1. Spins on REG_DUMMY_STATUS waiting for the Ping.
 *   2. Sends a payload to REG_DUMMY_TX.
 *   3. Exits cleanly.
 */

#include <stdint.h>

#define UART0_BASE 0x09000000
#define DUMMY_BASE 0x09005000

#define REG_DUMMY_STATUS (DUMMY_BASE + 0x00)
#define REG_DUMMY_TX (DUMMY_BASE + 0x04)
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
    uart_puts("Node 0: Pinger starting\n");

    // Send Ping
    *(volatile uint32_t *)REG_DUMMY_TX = 0x50494E47; // "PING"
    uart_puts("Node 0: Ping sent, waiting for Pong...\n");

    // Wait for Pong (Tests Yield-on-Read / BQL safety)
    while ((*(volatile uint32_t *)REG_DUMMY_STATUS & STATUS_DATA_READY) == 0) {
    }

    uart_puts("Node 0: Pong received! Test complete.\n");

  } else if (NODE_ID == 1) {
    uart_puts("Node 1: Ponger starting\n");

    // Wait for Ping (Tests Yield-on-Read / BQL safety)
    while ((*(volatile uint32_t *)REG_DUMMY_STATUS & STATUS_DATA_READY) == 0) {
    }
    uart_puts("Node 1: Ping received!\n");

    // Send Pong
    *(volatile uint32_t *)REG_DUMMY_TX = 0x504F4E47; // "PONG"
    uart_puts("Node 1: Pong sent! Test complete.\n");
  }

  while (1) {
    // Halt
  }

  return 0;
}
