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

    /* 2. Send 0xAA */
    mov r2, #0xAA
    str r2, [r0, #0x08]

    /* Wait for RX FIFO Not Empty (SR bit 2 = 0x04) */
wait_aa:
    ldr r2, [r0, #0x0c]
    tst r2, #0x04
    beq wait_aa

    /* Read back and verify */
    ldr r2, [r0, #0x08]
    cmp r2, #0xAA
    bne fail

    /* 3. Send 0xBB */
    mov r2, #0xBB
    str r2, [r0, #0x08]

wait_bb:
    ldr r2, [r0, #0x0c]
    tst r2, #0x04
    beq wait_bb

    ldr r2, [r0, #0x08]
    cmp r2, #0xBB
    bne fail

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
