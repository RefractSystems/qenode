# Chapter 2: MMIO and Registers

Memory-Mapped I/O (MMIO) is the interface through which software controls hardware. This chapter traces an MMIO operation from the physical silicon level up to the emulator code.

## 2.1 The Silicon Side
In a physical SoC, a peripheral exposes a set of **Registers**. Each register is a small amount of fast storage (usually 8, 16, or 32 bits) that is physically wired to the peripheral's internal logic.
*   **Base Address:** The start of the peripheral's memory region (e.g., `0x4000C000`).
*   **Offset:** The position of a specific register relative to the base (e.g., `+0x04` for the Transmit Data register).

## 2.2 The Firmware Side
In C firmware, we interact with these registers using pointers. Because the compiler might try to optimize away "redundant" writes to a memory location, we **must** use the `volatile` keyword.

```c
// Define the UART Transmit Data Register
#define UART0_BASE 0x4000C000
#define UART0_TXDATA (*((volatile uint32_t *)(UART0_BASE + 0x04)))

void uart_putc(char c) {
    UART0_TXDATA = c; // This 'store' instruction triggers the hardware
}
```

## 2.3 The Emulator Side (TCG Intercept)
In QEMU, there is no physical UART at `0x4000C000`. Instead, the emulator registers a memory region. When the guest CPU executes the store instruction above, the Tiny Code Generator (TCG) detects that the address falls within a registered MMIO region and intercepts the access.

TCG pauses the CPU simulation and calls a registered **callback function** in the peripheral plugin (e.g., in Rust):

```rust
fn handle_mmio_write(offset: u64, value: u64, size: u32) {
    match offset {
        0x04 => {
            // The guest wrote to the TXDATA register!
            // In VirtMCU, we might send this byte over a Zenoh topic.
            send_to_bus(value as u8);
        },
        _ => sim_warn!("Unhandled write to offset 0x{:x}", offset),
    }
}
```

## 2.4 Register Types
*   **RW (Read/Write):** Software can read and write the value (e.g., a configuration register).
*   **RO (Read-Only):** Software can only read (e.g., a status register).
*   **W1C (Write-1-to-Clear):** Writing a '1' to a bit clears it (common for interrupt flags).
*   **FIFO (First-In, First-Out):** A sequence of writes to the same address pushes data into a queue.

## 2.5 Exercises

### Exercise 2.1: The Volatile Trap
What happens if you remove the `volatile` keyword from the `UART0_TXDATA` definition and write a loop that sends "HELLO"? How might an aggressive compiler optimize this?

### Exercise 2.2: Endianness
If the CPU is Little-Endian and it writes a 32-bit value `0x12345678` to offset `0x00`, what byte does the emulator see at `offset + 0`?

## 2.6 Learning Objectives
After this chapter, you can:
1.  Define the relationship between base address and offset.
2.  Explain why `volatile` is mandatory for MMIO.
3.  Describe how QEMU/TCG intercepts a guest memory access.
