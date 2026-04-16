.section .text
.global _start
_start:
    ldr r0, =0x09000000
    mov r1, #'X'
    str r1, [r0]
loop:
    wfi
    b loop
