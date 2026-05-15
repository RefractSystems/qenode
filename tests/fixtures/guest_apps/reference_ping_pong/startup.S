/* ==============================================================================
 * startup.S
 * ============================================================================== */

.global _start
.type _start, %function
_start:
    // Setup stack pointer to the end of RAM (0x40000000 + 16MB)
    ldr sp, =0x41000000
    
    // Branch to main() in C
    bl main

    // If main returns, halt
.type halt, %function
halt:
    b halt
.size halt, . - halt
.size _start, . - _start
