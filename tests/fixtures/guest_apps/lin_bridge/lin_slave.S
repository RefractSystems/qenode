.section .text
.global _start
_start:
    ldr r0, =0x4006A000
    @ Enable Break detection
    ldr r1, [r0, #0x10]
    orr r1, r1, #(1 << 24) @ LBKDE
    str r1, [r0, #0x10]
    ldr r1, =((1 << 19) | (1 << 18)) @ TE | RE
    str r1, [r0, #0x18]

    @ Send 'R' to register with coordinator
    mov r2, #'R'
    str r2, [r0, #0x1C]

loop:
    @ Wait for Break
wait_break:
    ldr r1, [r0, #0x14]
    tst r1, #(1 << 31) @ LBKDIF
    beq wait_break
    @ Clear LBKDIF
    ldr r2, =(1 << 31)
    str r2, [r0, #0x14]
    
    @ Send marker '1'
    mov r3, #'1'
    str r3, [r0, #0x1C]
    
    @ Receive Sync
wait_sync:
    ldr r1, [r0, #0x14]
    tst r1, #(1 << 21) @ RDRF
    beq wait_sync
    ldr r2, [r0, #0x1C] @ Read 0x55
    
    @ Send marker '2'
    mov r3, #'2'
    str r3, [r0, #0x1C]

    @ Receive PID
wait_pid:
    ldr r1, [r0, #0x14]
    tst r1, #(1 << 21)
    beq wait_pid
    ldr r2, [r0, #0x1C] @ Read PID
    
    @ Send marker '3'
    mov r3, #'3'
    str r3, [r0, #0x1C]

    @ If PID is 0xC0, send 'S' (Slave response)
    cmp r2, #0xC0
    bne loop
    
    mov r2, #'S'
    str r2, [r0, #0x1C]
    
    b loop
