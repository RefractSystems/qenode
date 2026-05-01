# Chapter 12: Glossary of Architectural Terminology

This glossary serves as the definitive reference for the precise technical nomenclature utilized throughout the VirtMCU architectural documentation and source code. 

**Advanced High-performance Bus (AHB):**
A high-bandwidth interconnect architecture defined within the ARM AMBA specification. It serves as the primary data conduit linking the central processing unit to critical, high-speed subsystems such as physical memory controllers and Direct Memory Access (DMA) engines.

**Advanced Peripheral Bus (APB):**
A subsidiary bus architecture optimized for minimal power consumption and simplified interfacing. It connects the AHB to lower-speed hardware components (e.g., UARTs, Timers). Emulating the latency introduced by the AHB-to-APB bridge is a critical aspect of achieving temporal accuracy in VirtMCU.

**AddressSanitizer (ASan):**
A rigorous compiler-instrumentation module utilized heavily in the VirtMCU Continuous Integration pipeline. It shadows the application's memory space to definitively detect and report fatal memory violations, such as Use-After-Free (UAF) errors and buffer overflows.

**Architecture Decision Record (ADR):**
A formal technical document capturing the context, alternatives, and ultimate justification for a significant engineering choice within the VirtMCU project. These records ensure the preservation of architectural intent across the project's lifecycle.

**Big QEMU Lock (BQL):**
The foundational, global recursive mutex governing concurrency within the QEMU emulation engine. It mathematically guarantees that no background thread can mutate the state of the emulated hardware while the virtual CPU is concurrently executing instructions.

**Chandy-Misra-Bryant (CMB):**
The foundational algorithm underlying VirtMCU's conservative Parallel Discrete Event Simulation (PDES) model. It ensures global temporal causality by preventing any node from advancing its virtual clock until it is mathematically guaranteed not to receive a message from the simulated past.

**Device Tree Blob (DTB):**
The flattened, machine-readable binary payload containing the Directed Acyclic Graph (DAG) that describes the hardware topology of the system. It is ingested by QEMU's `arm-generic-fdt` machine to dynamically instantiate the required CPU cores and peripheral components.

**Executable and Linkable Format (ELF):**
The standardized binary container for compiled firmware. It categorizes machine code and data into strict domains (such as `.text`, `.data`, and `.bss`), which the emulator must meticulously map into the virtual address space to achieve Binary Fidelity.

**Memory-Mapped I/O (MMIO):**
The architectural paradigm whereby hardware control registers are exposed directly within the CPU's standard unified address space. QEMU's Tiny Code Generator intercepts memory access instructions targeting these addresses, redirecting them to the corresponding VirtMCU software callbacks.

**Parallel Discrete Event Simulation (PDES):**
The overarching theoretical framework utilized by VirtMCU to simulate highly concurrent, distributed networks of microcontrollers while completely eradicating host-dependent execution jitter and maintaining absolute global determinism.

**QEMU Object Model (QOM):**
The sophisticated runtime type system engineered into QEMU. It provides the object-oriented primitives—such as hierarchical inheritance, dynamic instantiation, and polymorphic dispatch—required to manage complex taxonomies of virtual hardware components within a C-based codebase.

**Tiny Code Generator (TCG):**
The Just-In-Time (JIT) compilation engine embedded within QEMU. It accelerates emulation by translating blocks of guest machine code into optimized native host instructions on the fly.
