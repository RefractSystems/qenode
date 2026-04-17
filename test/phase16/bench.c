#include <stdint.h>

#define UART0_BASE 0x09000000
#define UART0_DR   (*(volatile uint32_t *)(UART0_BASE + 0x00))
#define UART0_FR   (*(volatile uint32_t *)(UART0_BASE + 0x18))
#define FR_TXFF    (1 << 5)

void putc(char c) {
    while (UART0_FR & FR_TXFF);
    UART0_DR = c;
}

void puts(const char *s) {
    while (*s) putc(*s++);
}

void puthex(uint32_t v) {
    for (int i = 7; i >= 0; i--) {
        int nibble = (v >> (i * 4)) & 0xf;
        putc(nibble < 10 ? '0' + nibble : 'A' + nibble - 10);
    }
}

int main() {
    puts("BENCH START\r\n");
    
    // 10M iterations
    uint32_t iterations = 10000000; 
    uint32_t sum = 0;
    
    // Use volatile to prevent compiler from optimizing the loop away
    // or replacing it with a closed-form formula.
    volatile uint32_t *p_sum = &sum;

    for (uint32_t i = 0; i < iterations; i++) {
        *p_sum += i;
        *p_sum ^= (*p_sum << 3);
        *p_sum += 0x12345678;
    }
    
    puts("BENCH DONE: ");
    puthex(sum);
    puts("\r\n");
    
    // Signal end by printing a specific pattern
    puts("EXIT\r\n");
    
    while(1);
    return 0;
}
