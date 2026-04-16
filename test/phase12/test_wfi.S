.section .text
.global _start
_start:
    /* Enable UART: UARTCR = TXE | RXE | UARTEN */
    ldr r0, =0x09000000
    ldr r1, =0x301
    str r1, [r0, #0x30]

    /* Enable TX interrupt: UARTIMSC = TXIM */
    mov r1, #0x20
    str r1, [r0, #0x38]

    /* Write 'X' to trigger TX interrupt */
    mov r1, #'X'
    str r1, [r0]

loop:
    wfi
    b loop
