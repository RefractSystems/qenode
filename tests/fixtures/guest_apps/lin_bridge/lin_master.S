.section .text
.global _start
_start:
    ldr r0, =0x4006A000
    ldr r1, =((1 << 19) | (1 << 18)) @ TE | RE
    str r1, [r0, #0x18]
    
loop:
    @ 1. Send Break
    ldr r1, [r0, #0x18]
    orr r1, r1, #(1 << 0) @ SBK
    str r1, [r0, #0x18]
    bic r1, r1, #(1 << 0)
    str r1, [r0, #0x18]

    @ 2. Send Sync (0x55)
    mov r2, #0x55
    str r2, [r0, #0x1C]
    
    @ 3. Send PID (0xC0) - arbitrary
    mov r2, #0xC0
    str r2, [r0, #0x1C]
    
    @ Wait a bit
    mov r3, #0x100000
delay:
    subs r3, r3, #1
    bne delay
    
    b loop
