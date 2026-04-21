.global _start
_start:
    ldr r0, =0x60000000     /* bridge base address */
    ldr r1, =0x3E8      /* 16 million iterations */
loop:
    str r1, [r0]            /* write to bridge */
    ldr r2, [r0]            /* read from bridge */
    subs r1, r1, #1
    bne loop

    /* Done, print something or hang */
    ldr r3, =0x09000000     /* UART base (assuming PL011) */
    mov r4, #'O'
    str r4, [r3]
    mov r4, #'K'
    str r4, [r3]
    mov r4, #'\n'
    str r4, [r3]

end:
    b end
