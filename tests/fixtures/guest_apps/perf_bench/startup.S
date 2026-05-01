.section .text
.global _start
_start:
    ldr sp, =0x48000000 // 128MB RAM starts at 0x40000000
    bl main
    b .
