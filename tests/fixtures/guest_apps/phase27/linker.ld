MEMORY
{
    RAM (rwx) : ORIGIN = 0x40000000, LENGTH = 1M
}
SECTIONS
{
    .text : { *(.text*) } > RAM
    .data : { *(.data*) } > RAM
    .bss : { *(.bss*) } > RAM
}
