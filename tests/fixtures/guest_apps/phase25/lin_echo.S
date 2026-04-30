.section .text
.global _start

_start:
    /* S32K144 LPUART0 Base: 0x4006A000 */
    ldr r0, =0x4006A000
    
    /* Enable LIN Break detection in BAUD (0x10) */
    /* BAUD[LBKDE] = bit 24 */
    ldr r1, [r0, #0x10]
    orr r1, r1, #(1 << 24)
    str r1, [r0, #0x10]
    
    /* Enable TX/RX in CTRL (0x18) */
    /* CTRL[TE] = bit 19, CTRL[RE] = bit 18 */
    ldr r1, =((1 << 19) | (1 << 18))
    str r1, [r0, #0x18]

loop:
    /* 1. Check for LBKDIF (bit 31) in STAT (0x14) */
    ldr r1, [r0, #0x14]
    tst r1, #(1 << 31)
    beq no_break
    
    /* Clear LBKDIF (W1C) */
    ldr r2, =(1 << 31)
    str r2, [r0, #0x14]
    
    /* Wait for TDRE (bit 23) in STAT (0x14) to send 'B' */
wait_tx_b:
    ldr r2, [r0, #0x14]
    tst r2, #(1 << 23)
    beq wait_tx_b
    mov r2, #'B'
    str r2, [r0, #0x1C]

no_break:
    /* 2. Check for RDRF (bit 21) in STAT (0x14) */
    ldr r1, [r0, #0x14]
    tst r1, #(1 << 21)
    beq loop
    
    /* Read DATA (0x1C) */
    ldr r2, [r0, #0x1C]
    
    /* Echo it back: Wait for TDRE (bit 23) */
wait_tx_echo:
    ldr r1, [r0, #0x14]
    tst r1, #(1 << 23)
    beq wait_tx_echo
    str r2, [r0, #0x1C]
    
    b loop
