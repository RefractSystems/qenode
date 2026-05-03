# Volume I: Foundations of Silicon & Simulation

## The Physical Blueprint of Virtualization

Before we can build a "Matrix" for microcontrollers, we must understand the silicon reality they inhabit. Volume I is dedicated to the fundamental building blocks of embedded systems and how they are represented within a virtual environment.

In this volume, we move from the high-level academic baseline to the low-level physical pulse of the machine. We explore how CPUs boot, how they communicate with peripherals via Memory-Mapped I/O (MMIO), and how the entire topology of a System-on-Chip (SoC) is described through Device Trees.

---

## Volume Contents

### 1. [SoC Anatomy - The Physical Blueprint](01-soc-anatomy.md)
An architectural overview of the microcontroller. We examine the relationship between the CPU core, the bus matrix, and the peripheral registers.

### 2. [Memory-Mapped I/O (MMIO) and Registers](02-mmio-and-registers.md)
The language of hardware. We explore how software interacts with physical silicon through memory addresses and register bitfields.

### 3. [ELF, Firmware, and the Boot Sequence](03-elf-and-firmware-boot.md)
From binary to execution. We follow the lifecycle of a firmware image, from the ELF linker script to the reset vector and the first instruction of `main()`.

### 4. [Device Tree - The Topology of Silicon](04-device-tree.md)
The map of the machine. We analyze the Flattened Device Tree (FDT) format and how it allows us to dynamically define complex hardware models without re-compiling the emulator.

### 5. [Interrupts and Timers - The Pulse of the Machine](07-interrupts-and-timers.md)
Managing asynchronous events. We explore the Nested Vectored Interrupt Controller (NVIC) and the timers that drive the timing-critical logic of embedded systems.

### 6. [The QEMU Architecture - A Modern Emulator](05-qemu-architecture.md)
The engine of our universe. An introduction to QEMU's high-performance emulation strategy and why it is the SOTA choice for VirtMCU.

---

## Foundational Principles

### 1. Register Fidelity
Our virtual models must behave exactly like the physical silicon. Every bit in a virtual register must have the same reset value and side-effects as its physical counterpart.

### 2. Address-Space Isolation
We maintain a strict mapping of the SoC bus matrix. A firmware access to an unmapped address must result in a predictable "Data Abort," just as it would on real hardware.

### 3. The Power of the Device Tree
We treat hardware as data. By using Device Trees, we decouple the emulator's logic from the specific machine instance, enabling rapid prototyping of custom silicon.
