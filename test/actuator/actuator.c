/*
 * test/actuator/actuator.c
 *
 * Bare-metal firmware to test the zenoh-actuator device.
 */

#include <stdint.h>

#define UART0_BASE    0x09000000
#define ACTUATOR_BASE 0x50000000

#define REG_ACTUATOR_ID (ACTUATOR_BASE + 0x00)
#define REG_DATA_SIZE   (ACTUATOR_BASE + 0x04)
#define REG_GO          (ACTUATOR_BASE + 0x08)
#define REG_DATA_START  (ACTUATOR_BASE + 0x10)

void uart_putc(char c) {
    *(volatile uint32_t *)UART0_BASE = c;
}

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
    uart_puts("Actuator test firmware starting...\n");

    /* Send control signal 1: Actuator ID 42, Value 3.14 */
    uart_puts("Sending control signal 1 (ID=42, Val=3.14)...\n");
    
    *(volatile uint32_t *)REG_ACTUATOR_ID = 42;
    *(volatile uint32_t *)REG_DATA_SIZE   = 1;
    
    /* Write 3.14 as a double to DATA[0] */
    double val = 3.14;
    *(volatile double *)REG_DATA_START = val;
    
    /* Trigger! */
    *(volatile uint32_t *)REG_GO = 1;

    uart_puts("Control signal 1 sent.\n");

    /* Send control signal 2: Actuator ID 99, Values [1.0, 2.0, 3.0] */
    uart_puts("Sending control signal 2 (ID=99, Vals=[1.0, 2.0, 3.0])...\n");

    *(volatile uint32_t *)REG_ACTUATOR_ID = 99;
    *(volatile uint32_t *)REG_DATA_SIZE   = 3;
    
    ((volatile double *)REG_DATA_START)[0] = 1.0;
    ((volatile double *)REG_DATA_START)[1] = 2.0;
    ((volatile double *)REG_DATA_START)[2] = 3.0;

    *(volatile uint32_t *)REG_GO = 1;

    uart_puts("Control signal 2 sent.\n");

    uart_puts("Test complete. Spinning...\n");
    while (1) {
        delay(1000000);
    }

    return 0;
}
