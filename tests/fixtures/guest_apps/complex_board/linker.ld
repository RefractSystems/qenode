/* tests/fixtures/guest_apps/complex_board/linker.ld */
ENTRY(_start)
SECTIONS
{
    . = 0x40000000;
    .text : { *(.text) }
    .data : { *(.data) }
    .bss : { *(.bss) }
}
