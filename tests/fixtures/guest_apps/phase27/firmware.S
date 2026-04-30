
.global _start
_start:
    /* 1. Configure Receive Message in RAM via Interface */
    /* WRHS1: Frame ID = 20 (bits 0-10) */
    ldr r0, =0x09003400
    mov r1, #20
    str r1, [r0]

    /* WRHS2: Payload Length = 8 (16 bytes), Cycle Count = 0xFF (any) */
    ldr r0, =0x09003404
    ldr r1, =0x000800FF
    str r1, [r0]

    /* WRDS1: Write dummy data just for the TX test to send */
    ldr r0, =0x09003410
    ldr r1, =0xDEADC0DE
    str r1, [r0]

    /* IBCR: Transfer to msg_idx 1 */
    ldr r0, =0x09003500
    mov r1, #1
    str r1, [r0]

    /* 2. Start controller */
    ldr r0, =0x09003080
    mov r1, #4
    str r1, [r0]

wait_loop:
    /* 3. Check Message RAM Slot 1 for data */
    /* We'll use OBCR to read it back into ORDS */
    ldr r0, =0x09003700
    mov r1, #1
    str r1, [r0]

    /* Check ORDS1 for our expected RX value 0xCAFEBABE */
    ldr r0, =0x09003610
    ldr r1, [r0]
    ldr r2, =0xCAFEBABE
    cmp r1, r2
    bne wait_loop

    /* 4. Signal success via WRHS3 */
    ldr r0, =0x09003408
    ldr r1, =0x12345678
    str r1, [r0]

loop:
    nop
    b loop
