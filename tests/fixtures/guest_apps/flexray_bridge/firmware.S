
.global _start
_start:
    /* Print BOOT to UART */
    ldr r2, =0x09000000
    mov r3, #66  /* 'B' */
    str r3, [r2]
    mov r3, #79  /* 'O' */
    str r3, [r2]
    str r3, [r2]
    mov r3, #84  /* 'T' */
    str r3, [r2]
    mov r3, #10 /* '\n' */
    str r3, [r2]

    /* 1. Configure Message RAM via Interface */

    /* Slot 0: Frame ID = 10, for TX test */
    ldr r0, =0x09003400
    mov r1, #10
    str r1, [r0]
    ldr r0, =0x09003404
    ldr r1, =0x000800FF
    str r1, [r0]
    ldr r0, =0x09003410
    ldr r1, =0xDEC0ADDE
    str r1, [r0]
    ldr r0, =0x09003500
    mov r1, #0
    str r1, [r0]

    /* Slot 1: Frame ID = 20, for RX test */
    ldr r0, =0x09003400
    mov r1, #20
    str r1, [r0]
    ldr r0, =0x09003404
    ldr r1, =0x000800FF
    str r1, [r0]
    ldr r0, =0x09003500
    mov r1, #1
    str r1, [r0]

    /* Slot 2: Frame ID = 30, for loopback */
    ldr r0, =0x09003400
    mov r1, #30
    str r1, [r0]
    ldr r0, =0x09003404
    ldr r1, =0x000800FF
    str r1, [r0]
    ldr r0, =0x09003500
    mov r1, #2
    str r1, [r0]

    /* 2. Global Config */
    ldr r0, =0x09003000
    ldr r1, =0x00000001
    str r1, [r0]

loop:
    /* Check Slot 1 (RX) Status */
    ldr r0, =0x09003408
    ldr r1, [r0]
    tst r1, #1
    beq skip_rx

    /* Read Slot 1 (RX) Data */
    ldr r0, =0x09003410
    ldr r1, [r0]

    /* Write to UART 0x09000000 - 4 bytes individually */
    ldr r2, =0x09000000
    /* Byte 0 */
    strb r1, [r2]
    /* Byte 1 */
    lsr r3, r1, #8
    strb r3, [r2]
    /* Byte 2 */
    lsr r3, r1, #16
    strb r3, [r2]
    /* Byte 3 */
    lsr r3, r1, #24
    strb r3, [r2]

    /* Clear Status */
    ldr r0, =0x09003408
    mov r1, #0
    str r1, [r0]

skip_rx:
    b loop
