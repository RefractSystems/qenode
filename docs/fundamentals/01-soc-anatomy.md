# Chapter 1: SoC Anatomy

To simulate an MCU, one must first understand what an MCU is. A modern microcontroller is typically a **System-on-Chip (SoC)**—a single silicon die containing a CPU core and a collection of peripherals.

## 1.1 The Address Space View

The most fundamental concept in SoC architecture is the **Memory Map**. To the CPU, the world is a linear array of addresses. Some addresses point to physical RAM, some to Flash memory, and some—the **Memory Mapped I/O (MMIO)** regions—point to peripheral registers.

### Diagram: Typical Cortex-M Memory Map

```text
+-----------------------+ 0xFFFFFFFF
|   Private Peripheral  |
|      Bus (Internal)   |
+-----------------------+ 0xE0100000
|                       |
|      External RAM     |
+-----------------------+ 0x60000000
|                       |
|      Peripherals      | <-- MMIO Region (e.g., UART at 0x4000C000)
+-----------------------+ 0x40000000
|                       |
|          SRAM         |
+-----------------------+ 0x20000000
|                       |
|      Code / Flash     |
+-----------------------+ 0x00000000
```

## 1.2 Core Components

### The CPU Core
The "brain" that executes instructions (e.g., ARM Cortex-M4, RISC-V RV32IMAC). In simulation, this is handled by the QEMU front-end.

### Peripheral Bus (AHB/APB)
The "highway" that connects the CPU to peripherals.
*   **AHB (Advanced High-performance Bus):** For high-bandwidth components like RAM and Flash.
*   **APB (Advanced Peripheral Bus):** For lower-bandwidth peripherals like UART, SPI, and Timers.

### Interrupt Controller (NVIC/GIC)
The "secretary" that manages hardware signals (Interrupt Requests, or IRQs). When a peripheral needs attention (e.g., a byte arrived on the UART), it signals the Interrupt Controller, which then pauses the CPU to run an Interrupt Service Routine (ISR).

### Memory Controller
Manages the translation between the CPU's address bus and the actual physical storage (SRAM/Flash).

## 1.3 Why 0x40000000 Matters

In the diagram above, the region starting at `0x40000000` is designated for peripherals. If a datasheet says "the UART0 base address is 0x4000C000," it means that any Load or Store instruction the CPU executes targeting that address will be routed by the bus to the UART0 peripheral, rather than to RAM.

## 1.4 Exercises

### Exercise 1.1: Map Analysis
Find the datasheet for an STM32F405. Locate the "Memory Map" section. At what address does the `GPIOA` peripheral live? How many bytes of address space does it occupy?

### Exercise 1.2: The "Hole"
What happens if the CPU attempts to read from an address that is not mapped to any memory or peripheral (a "hole" in the map)? Research "Bus Fault" or "Data Abort".

## 1.5 Learning Objectives
After this chapter, you can:
1.  Explain the concept of a memory-mapped SoC.
2.  Identify the role of the AHB/APB buses.
3.  Visualize the path of a memory access from the CPU to a peripheral.
