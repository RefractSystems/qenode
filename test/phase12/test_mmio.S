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
    b .                /* IRQ */
    b .                /* FIQ */

_start:
    /* Set VBAR and MVBAR to our vector table */
    ldr r0, =vector_table
    mcr p15, 0, r0, c12, c0, 0  /* VBAR */
    mcr p15, 0, r0, c12, c0, 1  /* MVBAR */

    ldr r0, =0x10000000
    ldr r1, =0xDEADBEEF
    str r1, [r0]
    ldr r0, =0x10000004
    ldr r1, =0xCAFEBABE
    str r1, [r0]

    /* Exit via semihosting if possible, but we don't enable it usually */
    /* So it will likely just hit the loop below */
    
loop:
    b loop
