/* ==============================================================================
 * hello.S
 *
 * A bare-metal assembly program to say "HI" over UART for Phase 1 testing.
 * 
 * When the CPU resets, it starts executing instructions from the entry point.
 * Without an Operating System to provide printf(), we must talk to the hardware directly.
 * We do this using Memory Mapped I/O (MMIO).
 * ============================================================================== */

.global _start
_start:
    /* 
     * Load the physical address of the PL011 UART into register r0.
     * This address MUST match what we defined in minimal.dts!
     */
    ldr r0, =0x09000000
    
    /* Move the ASCII character 'H' into register r1 */
    mov r1, #'H'
    
    /* Store the word in r1 into the memory address pointed to by r0.
     * For the PL011 UART in QEMU, writing a 32-bit word to the base address
     * transmits a character.
     */
    str r1, [r0]
    
    /* Repeat for 'I' */
    mov r1, #'I'
    str r1, [r0]
    
    /* Repeat for a newline character */
    mov r1, #'\n'
    str r1, [r0]

loop:
    /* 
     * Wait For Interrupt (wfi) puts the CPU to sleep to save power.
     * Since we haven't configured any interrupts, it will sleep forever.
     */
    nop
    
    /* Infinite loop just in case it wakes up */
    b loop
