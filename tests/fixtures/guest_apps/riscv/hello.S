.section .text
.global _start

_start:
    # Set stack pointer (optional for this simple loop)
    # la sp, _stack_top

    # NS16550A UART base address (0x10000000 on the QEMU RISC-V virt machine)
    li t0, 0x10000000

    # Message "HI RV\n"
    li t1, 'H'
    sb t1, 0(t0)
    li t1, 'I'
    sb t1, 0(t0)
    li t1, ' '
    sb t1, 0(t0)
    li t1, 'R'
    sb t1, 0(t0)
    li t1, 'V'
    sb t1, 0(t0)
    li t1, '\n'
    sb t1, 0(t0)

loop:
    nop
    j loop
