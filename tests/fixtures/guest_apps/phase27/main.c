#include <stdint.h>

#define FLEXRAY_BASE 0x09003000
#define CCRR (FLEXRAY_BASE + 0x80)
#define CCSV (FLEXRAY_BASE + 0x84)

void main(void) {
  volatile uint32_t *ccrr = (uint32_t *)CCRR;
  volatile uint32_t *ccsv = (uint32_t *)CCSV;

  // Start controller
  *ccrr = 0x4;

  // Wait for some cycles
  for (int i = 0; i < 1000000; i++) {
    asm volatile("nop");
  }

  // Done
  while (1)
    ;
}
