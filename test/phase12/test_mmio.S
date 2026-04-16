.section .text
.global _start
_start:
    ldr r0, =0x10000000
    ldr r1, =0xDEADBEEF
    str r1, [r0]
    ldr r0, =0x10000004
    ldr r1, =0xCAFEBABE
    str r1, [r0]
loop:
    b loop
