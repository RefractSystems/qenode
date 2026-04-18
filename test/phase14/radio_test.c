/*
 * test/phase14/radio_test.c
 *
 * Bare-metal firmware to test the zenoh-802154 radio device.
 * Uses PL011 UART for logging.
 */

#include <stdint.h>

#define UART0_BASE    0x09000000
#define RADIO_BASE    0x09001000

/* PL011 Registers */
#define UARTDR        (UART0_BASE + 0x00)
#define UARTFR        (UART0_BASE + 0x18)
#define UARTFR_TXFF   (1 << 5)

/* Radio Registers */
#define REG_RADIO_TX_DATA   (RADIO_BASE + 0x00)
#define REG_RADIO_TX_LEN    (RADIO_BASE + 0x04)
#define REG_RADIO_TX_GO     (RADIO_BASE + 0x08)
#define REG_RADIO_RX_DATA   (RADIO_BASE + 0x0C)
#define REG_RADIO_RX_LEN    (RADIO_BASE + 0x10)
#define REG_RADIO_STATUS    (RADIO_BASE + 0x14)
#define REG_RADIO_RX_RSSI   (RADIO_BASE + 0x18)

/* New registers for 14.5 (proposed) */
#define REG_RADIO_PAN_ID    (RADIO_BASE + 0x20)
#define REG_RADIO_SHORT_ADDR (RADIO_BASE + 0x24)

#define STATUS_RX_READY     0x01
#define STATUS_TX_DONE      0x02

void uart_putc(char c) {
    while (*(volatile uint32_t *)UARTFR & UARTFR_TXFF);
    *(volatile uint32_t *)UARTDR = c;
}

void uart_puts(const char *s) {
    while (*s) {
        uart_putc(*s++);
    }
}

void uart_puthex(uint32_t v) {
    const char *hex = "0123456789ABCDEF";
    for (int i = 7; i >= 0; i--) {
        uart_putc(hex[(v >> (i * 4)) & 0xF]);
    }
}

void delay(int count) {
    for (int i = 0; i < count; i++) {
        asm volatile("nop");
    }
}

int main() {
    uart_puts("\nRadio test firmware starting...\n");

    /* Set PAN ID and Short Address */
    uart_puts("Setting PAN ID 0xABCD and Short Addr 0x1234...\n");
    *(volatile uint32_t *)REG_RADIO_PAN_ID = 0xABCD;
    *(volatile uint32_t *)REG_RADIO_SHORT_ADDR = 0x1234;

    /* Prepare a dummy 802.15.4 frame */
    uint8_t frame[] = {
        0x41, 0x88,         /* Frame Control: Data, Ack Request, Pan ID Compression */
        0x01,               /* Sequence Number */
        0xCD, 0xAB,         /* Dest PAN ID: 0xABCD */
        0x34, 0x12,         /* Dest Addr: 0x1234 */
        0x78, 0x56,         /* Source Addr: 0x5678 */
        'P', 'I', 'N', 'G'
    };

    uart_puts("Sending packet...\n");
    for (int i = 0; i < sizeof(frame); i++) {
        *(volatile uint32_t *)REG_RADIO_TX_DATA = (uint32_t)frame[i];
    }
    *(volatile uint32_t *)REG_RADIO_TX_LEN = (uint32_t)sizeof(frame);
    *(volatile uint32_t *)REG_RADIO_TX_GO = 1;

    /* Wait for TX_DONE */
    while (!(*(volatile uint32_t *)REG_RADIO_STATUS & STATUS_TX_DONE)) {
        delay(100);
    }
    /* Clear TX_DONE */
    *(volatile uint32_t *)REG_RADIO_STATUS = STATUS_TX_DONE;
    uart_puts("Packet sent successfully.\n");

    uart_puts("Waiting for RX...\n");
    while (1) {
        uint32_t status = *(volatile uint32_t *)REG_RADIO_STATUS;
        if (status & STATUS_RX_READY) {
            uint32_t len = *(volatile uint32_t *)REG_RADIO_RX_LEN;
            int8_t rssi = (int8_t)*(volatile uint32_t *)REG_RADIO_RX_RSSI;
            
            uart_puts("Received packet! Len=");
            uart_puthex(len);
            uart_puts(" RSSI=");
            uart_puthex((uint32_t)rssi);
            uart_puts("\nData: ");
            
            for (uint32_t i = 0; i < len; i++) {
                uint8_t b = (uint8_t)*(volatile uint32_t *)REG_RADIO_RX_DATA;
                if (b >= 32 && b <= 126) uart_putc((char)b);
                else uart_putc('.');
            }
            uart_puts("\n");
            
            /* Clear RX_READY */
            *(volatile uint32_t *)REG_RADIO_STATUS = STATUS_RX_READY;
        }
        delay(1000);
    }

    return 0;
}
