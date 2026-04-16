/* test/actuator/startup.S */
.global _start
_start:
    /* Setup stack pointer */
    ldr sp, =0x41000000
    /* Jump to C main */
    bl main
    /* Infinite loop if main returns */
1:  b 1b
