/* tests/fixtures/guest_apps/actuator/startup.S */
.global _start
_start:
    /* Enable FPU (CP10, CP11) */
    mrc p15, 0, r0, c1, c0, 2
    orr r0, r0, #0x00F00000
    mcr p15, 0, r0, c1, c0, 2
    isb
    mov r0, #0x40000000
    vmsr fpexc, r0

    /* Setup stack pointer */
    ldr sp, =0x41000000
    /* Jump to C main */
    bl main
    /* Infinite loop if main returns */
1:  b 1b
