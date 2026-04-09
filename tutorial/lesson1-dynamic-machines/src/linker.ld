/* ==============================================================================
 * linker.ld
 * 
 * Tells the compiler where to put our code in memory.
 * ============================================================================== */

/* The entry point of our program is the _start label in hello.S */
ENTRY(_start)

SECTIONS
{
    /* 
     * The dot (.) represents the "current address counter".
     * We set it to 0x40000000, which is exactly where we told QEMU 
     * our RAM starts in the minimal.dts file!
     */
    . = 0x40000000;
    
    /* Put all .text sections (the actual executable code) here */
    .text : { *(.text) }
}
