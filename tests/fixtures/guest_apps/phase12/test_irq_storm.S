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
    ldr r0, =vector_table
    mcr p15, 0, r0, c12, c0, 0
    ldr r0, =0x09000000
    ldr r1, =0x301
    str r1, [r0, #0x30]
    mov r1, #0x20
    str r1, [r0, #0x38]
    cpsie i
    mov r1, #'X'
    str r1, [r0]
loop:
    b loop

irq_handler:
    ldr r0, =0x09000000
    mov r1, #0x20
    str r1, [r0, #0x44]
    mov r1, #'X'
    str r1, [r0]
    subs pc, lr, #4
