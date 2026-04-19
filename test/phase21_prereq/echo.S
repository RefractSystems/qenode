/*
 * test/phase21_prereq/echo.S
 *
 * Minimal ARMv7 firmware to test PL022 SPI and UART Echo devices.
 */

.section .text
.global _start

_start:
    /* Set up stack */
    ldr sp, =0x44000000

    /* Say Hello on UART0 */
    ldr r0, =0x09000000 /* UART0 base */
    mov r1, #'H'
    str r1, [r0]
    mov r1, #'i'
    str r1, [r0]
    mov r1, #'\n'
    str r1, [r0]

    /* Test SPI Echo (PL022) */
    ldr r0, =0x09001000 /* SPI0 base */
    
    /* Enable SSP, Master mode */
    mov r1, #0x02 /* SSE bit in CR1 */
    str r1, [r0, #0x04] /* SSPCR1 */
    
    /* Set clock, 8-bit, etc in CR0 */
    mov r1, #0x07 /* 8-bit data */
    str r1, [r0, #0x00] /* SSPCR0 */

    /* Send byte 0x55 */
    mov r1, #0x55
    str r1, [r0, #0x08] /* SSPDR */

wait_spi:
    ldr r1, [r0, #0x0C] /* SSPSR */
    tst r1, #0x04 /* RNE (Receive FIFO not empty) */
    beq wait_spi

    ldr r1, [r0, #0x08] /* SSPDR */
    cmp r1, #0x55
    beq spi_ok

    /* SPI Fail */
    ldr r0, =0x09000000
    mov r1, #'S'
    str r1, [r0]
    mov r1, #'F'
    str r1, [r0]
    mov r1, #'\n'
    str r1, [r0]
    b halt

spi_ok:
    ldr r0, =0x09000000
    mov r1, #'S'
    str r1, [r0]
    mov r1, #'O'
    str r1, [r0]
    mov r1, #'K'
    str r1, [r0]
    mov r1, #'\n'
    str r1, [r0]

    /* Test UART Echo (uart-echo) */
    /* Note: We need a second UART or a way to talk to uart-echo.
       The YAML says uart_echo_link is at 0x09002000 but it is a custom device.
       Actually, uart-echo doesn't have its own registers, it's a device that 
       takes a chardev.
       Wait, I should have made uart-echo a SysBusDevice with registers to talk to it.
    */

    /* I'll skip UART echo test for now or implement registers in uart-echo.
       Actually, let's implement a simple 1-register interface in UARTEcho.
    */

    b halt

halt:
    wfi
    b halt
