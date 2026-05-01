#include <stdint.h>

#define UART0_BASE 0x09000000
#define UART0_DR   (*(volatile uint32_t *)(UART0_BASE + 0x00))
#define UART0_FR   (*(volatile uint32_t *)(UART0_BASE + 0x18))
#define FR_TXFF    (1 << 5)

static void uart_putc(char c) {
    while (UART0_FR & FR_TXFF);
    UART0_DR = c;
}

static void uart_puts(const char *s) {
    while (*s) uart_putc(*s++);
}

static void uart_puthex(uint32_t v) {
    for (int i = 7; i >= 0; i--) {
        int nibble = (v >> (i * 4)) & 0xf;
        uart_putc(nibble < 10 ? '0' + nibble : 'A' + nibble - 10);
    }
}

/* ARM Generic Timer — virtual counter (CNTVCT), AArch32 encoding.
 * In QEMU icount shift=0, this counter runs at CNTFRQ_EL0 Hz tied to the
 * virtual clock, so delta_cntvct == delta_virtual_ns when CNTFRQ == 1 GHz. */
static uint64_t read_cntvct(void) {
    uint32_t lo, hi;
    asm volatile("mrrc p15, 1, %0, %1, c14" : "=r"(lo), "=r"(hi));
    return ((uint64_t)hi << 32) | lo;
}

/* Counter frequency register — used to validate QEMU timer assumptions. */
static uint32_t read_cntfrq(void) {
    uint32_t v;
    asm volatile("mrc p15, 0, %0, c14, c0, 0" : "=r"(v));
    return v;
}

int main(void) {
    uart_puts("BENCH START\r\n");

    /* Emit counter frequency so bench.py can validate the 1 GHz assumption. */
    uart_puts("CNTFRQ: ");
    uart_puthex(read_cntfrq());
    uart_puts("\r\n");

    uint32_t sum = 0;
    volatile uint32_t *p_sum = &sum;

    uint64_t t0 = read_cntvct();

    for (uint32_t i = 0; i < 10000000; i++) {
        *p_sum += i;
        *p_sum ^= (*p_sum << 3);
        *p_sum += 0x12345678;
    }

    uint64_t t1 = read_cntvct();
    uint64_t cycles = t1 - t0;

    uart_puts("BENCH DONE: ");
    uart_puthex(sum);
    uart_puts("\r\n");

    /* Exact counter delta — deterministic across icount runs. */
    uart_puts("CYCLES: ");
    uart_puthex((uint32_t)(cycles >> 32));
    uart_puthex((uint32_t)cycles);
    uart_puts("\r\n");

    uart_puts("EXIT\r\n");

    while (1) {
        for (volatile int i = 0; i < 1000; i++);
    }
    return 0;
}
