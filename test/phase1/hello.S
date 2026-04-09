.global _start
_start:
    ldr r0, =0x09000000
    mov r1, #'H'
    str r1, [r0]
    mov r1, #'I'
    str r1, [r0]
    mov r1, #'\n'
    str r1, [r0]
loop:
    wfi
    b loop
