// Basic Interactive UART Echo Firmware for ARM Cortex-A15
// Hardware: PL011 UART at 0x09000000

.equ UART0_BASE, 0x09000000
.equ UART0_DR,   (UART0_BASE + 0x00) // Data Register
.equ UART0_FR,   (UART0_BASE + 0x18) // Flag Register
.equ UART0_CR,   (UART0_BASE + 0x30) // Control Register

.equ FR_TXFF,    (1 << 5) // Transmit FIFO full
.equ FR_RXFE,    (1 << 4) // Receive FIFO empty

.global _start
_start:
    // Initialize UART (Enable UART, TX, RX)
    ldr r1, =UART0_CR
    ldr r0, =0x301  // UARTEN (bit 0) | TXE (bit 8) | RXE (bit 9)
    str r0, [r1]

    // Print initial welcome message
    ldr r1, =welcome_msg
print_welcome:
    ldrb r0, [r1], #1
    cmp r0, #0
    beq loop
    bl putc
    b print_welcome

loop:
    // 1. Wait until a character is received (RXFE is 0)
    ldr r1, =UART0_FR
wait_rx:
    ldr r2, [r1]
    tst r2, #FR_RXFE
    bne wait_rx        // Branch if empty

    // 2. Read the character from Data Register
    ldr r1, =UART0_DR
    ldr r0, [r1]

    // 3. Echo it back
    bl putc

    // 4. If character is '\r' (CR), also send '\n' (LF)
    cmp r0, #13
    bne loop
    mov r0, #10
    bl putc
    b loop

putc:
    // Wait until transmit FIFO is not full (TXFF is 0)
    ldr r2, =UART0_FR
wait_tx:
    ldr r3, [r2]
    tst r3, #FR_TXFF
    bne wait_tx

    // Write character
    ldr r2, =UART0_DR
    str r0, [r2]
    bx lr

.section .data
welcome_msg:
    .asciz "Interactive UART Echo Ready.\r\nType something: "
    .align 2
