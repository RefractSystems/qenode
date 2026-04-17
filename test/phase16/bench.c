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

// Read the ARM Generic Timer Counter (CNTVCT_EL0)
// This requires the generic timer to be present, which it is on cortex-a15
uint64_t read_cntvct(void) {
    uint32_t low, high;
    // mrrc p15, 1, Rt, Rt2, c14 
    asm volatile("mrrc p15, 1, %0, %1, c14" : "=r" (low), "=r" (high));
    return ((uint64_t)high << 32) | low;
}

int main() {
    puts("BENCH START\r\n");
    
    // 10M iterations
    uint32_t iterations = 10000000; 
    uint32_t sum = 0;
    
    volatile uint32_t *p_sum = &sum;

    uint64_t start_cycles = read_cntvct();

    for (uint32_t i = 0; i < iterations; i++) {
        *p_sum += i;
        *p_sum ^= (*p_sum << 3);
        *p_sum += 0x12345678;
    }
    
    uint64_t end_cycles = read_cntvct();
    uint64_t total_cycles = end_cycles - start_cycles;

    puts("BENCH DONE: ");
    puthex(sum);
    puts("\r\n");
    
    // Print the exact cycle count for the determinism check
    puts("CYCLES: ");
    puthex((uint32_t)(total_cycles >> 32));
    puthex((uint32_t)total_cycles);
    puts("\r\n");
    
    puts("EXIT\r\n");
    
    while(1) {
        for(volatile int i=0; i<1000; i++);
    }
    return 0;
}
