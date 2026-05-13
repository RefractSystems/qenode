/* linker.ld */
ENTRY(_start)

MEMORY
{
  RAM (xrw) : ORIGIN = 0x40000000, LENGTH = 16M
}

SECTIONS
{
  .text :
  {
    KEEP(*(.isr_vector))
    *(.text*)
    *(.rodata*)
  } > RAM

  .data :
  {
    *(.data*)
  } > RAM

  .bss :
  {
    __bss_start__ = .;
    *(.bss*)
    *(COMMON)
    __bss_end__ = .;
  } > RAM

  .stack :
  {
    . = ALIGN(8);
    __stack_start__ = .;
    . = . + 0x1000; /* 4KB stack */
    __stack_end__ = .;
  } > RAM
}
