# Chapter 3: Executable Binaries and the Boot Vector

## 3.1 The Philosophy of Binary Fidelity

The central axiom of the VirtMCU emulation strategy is **Binary Fidelity**: the stipulation that the compiled firmware artifact deployed to physical silicon must execute without modification within the simulation environment. Emulators that necessitate "simulation-only" compiler flags, altered linker scripts, or mocked startup routines inherently compromise the validity of the testing process. To achieve true cyber-physical equivalence, the emulator must ingest and execute the standard Executable and Linkable Format (ELF) file precisely as a hardware bootloader would.

## 3.2 The Executable and Linkable Format (ELF)

In modern embedded systems, firmware is overwhelmingly packaged as an ELF binary. The ELF specification provides a rigidly structured taxonomy for organizing machine code, initialized data, and execution metadata. Rather than viewing the binary as a monolithic block of instructions, the ELF standard compartmentalizes the program into functional domains known as **Sections**, which are subsequently mapped into memory **Segments** by the loader.

### 3.2.1 Section Taxonomy

A robust understanding of ELF sections is critical for diagnosing initialization failures:

*   **The `.text` Section:** This domain houses the executable machine instructions. In an embedded context, this section is universally targeted for non-volatile storage (Flash memory) and is executed in-place (XIP). It is strictly read-only.
*   **The `.data` Section:** This section contains statically allocated variables that possess a non-zero initial value (e.g., `int config_mode = 5;`). A paradox arises here: the variable's value must be mutable at runtime (requiring placement in SRAM), yet its initial value must persist across power cycles (requiring placement in Flash).
*   **The `.bss` Section:** Representing "Block Started by Symbol," this section is reserved for statically allocated variables that are uninitialized or explicitly initialized to zero. To conserve ROM space, the `.bss` section occupies no physical bytes in the ELF file's storage payload; it merely specifies the required dimensions in SRAM that must be zeroed before `main()` executes.

## 3.3 The Linker Script: Dictating the Topology

In a desktop operating system, the kernel's loader dynamically assigns memory to an application. In bare-metal embedded systems, there is no kernel. The memory topology is immutable and must be statically defined at compile time. This is the sole responsibility of the **Linker Script** (`.ld`).

The linker script maps the abstract ELF sections onto the physical memory map detailed in the silicon's datasheet. It explicitly decrees that the `.text` section resides in Flash, while the `.bss` and `.data` sections reside in SRAM. 

Crucially, the linker script resolves the `.data` paradox by utilizing Load Memory Addresses (LMA) and Virtual Memory Addresses (VMA). It instructs the linker to store the `.data` payload in Flash (the LMA) but configures the variable pointers to reference SRAM (the VMA). 

## 3.4 The Boot Sequence and the Vector Table

Upon power-on reset, the processor does not arbitrarily begin executing code. ARM Cortex-M architectures define a strict hardware protocol based on the **Vector Table**, typically located at the absolute base of the Flash memory map (e.g., `0x0000_0000` or `0x0800_0000`).

The first two words of the Vector Table are the genesis of execution:
1.  **Word 0 (Initial SP):** The hardware unconditionally loads this value into the Main Stack Pointer (MSP).
2.  **Word 1 (Reset Vector):** The hardware loads this address into the Program Counter (PC) and begins execution.

This Reset Vector invariably points to the **Startup Routine** (often written in assembly or naked C). The startup routine's primary mandate is to construct the C runtime environment before invoking `main()`. It must manually iterate through the Flash memory, copying the `.data` payload into SRAM, and subsequently iterate through the `.bss` region, zeroing out the memory.

## 3.5 VirtMCU and the ELF Loader

When VirtMCU (via QEMU's `arm-generic-fdt` machine) is provided with an ELF file via the `-kernel` argument, it acts as a surrogate bootloader. QEMU parses the ELF headers, extracts the memory segments, and injects them directly into the emulated memory spaces dictated by the Device Tree. It then extracts the Entry Point (the Reset Vector) and initializes the virtual CPU's Program Counter.

If the firmware's Linker Script assumes Flash memory begins at `0x0800_0000`, but the VirtMCU Device Tree omits this memory region or maps it elsewhere, QEMU will inject the code into a void. Upon boot, the virtual CPU will attempt an instruction fetch from an unmapped address, resulting in an immediate and fatal Data Abort before the first line of startup code can even execute.

## 3.6 Summary

The ELF binary is a structured orchestration of code and data, marshaled into physical memory locations by the Linker Script. Understanding the sequence of events from the hardware reading the Reset Vector to the software copying the `.data` section is vital. In emulation, discrepancies between the linker's assumptions and the emulator's memory map are a primary source of catastrophic initialization failures.

## 3.7 Exercises

1.  **The `.data` Initialization Failure:** Imagine a scenario where a firmware engineer accidentally removes the routine that copies the `.data` section from Flash to SRAM in the startup assembly. What specific runtime behaviors would you observe when evaluating the state of global variables?
2.  **Vector Table Alignment:** According to the ARMv7-M Architecture Reference Manual, the addresses stored in the Vector Table (including the Reset Vector) must have their Least Significant Bit (LSB) set to 1. Explain the architectural reasoning behind this requirement and what happens if a bootloader violates it.
3.  **Emulation Discrepancy:** QEMU's ELF loader typically bypasses the hardware-level Vector Table fetch, forcibly setting the PC to the ELF's declared entry point and directly initializing the Stack Pointer. Discuss the advantages of this approach for simulation speed, and identify potential edge-case bugs in firmware startup routines that this bypass might obscure.