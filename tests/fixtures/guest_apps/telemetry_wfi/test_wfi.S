.section .text
.global _start
.balign 32
vector_table:
    b _start           /* Reset */
    b .                /* Undefined */
    b .                /* SWI */
    b .                /* Prefetch Abort */
    b .                /* Data Abort */
    b .                /* Reserved */
    b irq_handler      /* IRQ */
    b .                /* FIQ */

_start:
    /* Set VBAR and MVBAR to our vector table */
    ldr r0, =vector_table
    mcr p15, 0, r0, c12, c0, 0  /* VBAR */
    mcr p15, 0, r0, c12, c0, 1  /* MVBAR */

    /* Enable UART: UARTCR = TXE | RXE | UARTEN */
    ldr r0, =0x09000000
    ldr r1, =0x301
    str r1, [r0, #0x30]

    /* Enable TX interrupt: UARTIMSC = TXIM */
    mov r1, #0x20
    str r1, [r0, #0x38]

    /* Enable IRQs in CPSR */
    cpsie i

    /* Write 'X' to trigger TX interrupt */
    mov r1, #'X'
    str r1, [r0]

loop:
    wfi
    b loop

irq_handler:
    /* Clear UART TX interrupt by writing to UARTICR */
    ldr r0, =0x09000000
    mov r1, #0x20
    str r1, [r0, #0x44]
    
    subs pc, lr, #4
