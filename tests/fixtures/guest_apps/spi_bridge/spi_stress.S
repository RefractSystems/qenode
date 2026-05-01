.section .text
.global _start

_start:
    /* Base addresses */
    ldr r0, =0x09005000 /* PL022 base */
    ldr r1, =0x09000000 /* UART0 base */

    /* 1. Initialize PL022 */
    /* CR0: 8-bit data (0x07) */
    mov r2, #0x07
    str r2, [r0, #0x00]

    /* CPSR: Clock prescaler (2) */
    mov r2, #0x02
    str r2, [r0, #0x10]

    /* CR1: Enable (SSE=0x02) */
    mov r2, #0x02
    str r2, [r0, #0x04]

    /* Setup loop counter */
    ldr r3, =10000

loop_start:
    cmp r3, #0
    beq success

    /* 2. Send value (use lower 8 bits of counter) */
    and r2, r3, #0xFF
    str r2, [r0, #0x08]

wait_rx:
    ldr r2, [r0, #0x0c]
    tst r2, #0x04
    beq wait_rx

    /* Read back */
    ldr r2, [r0, #0x08]
    /* For stress test, we might not care about verifying exact value here,
       or we can assume echo server echoes it back. */
    and r4, r3, #0xFF
    cmp r2, r4
    bne fail

    sub r3, r3, #1
    b loop_start

success:
    /* Print 'P' to UART */
    mov r2, #0x50
    str r2, [r1]
    wfi
    b success

fail:
    /* Print 'F' to UART */
    mov r2, #0x46
    str r2, [r1]
    wfi
    b fail
