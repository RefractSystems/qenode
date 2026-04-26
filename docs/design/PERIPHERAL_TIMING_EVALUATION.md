# Peripheral Timing Fidelity: Options, Compromises, and Industry Context

## 1. Introduction: The Problem of Immediate Execution
In the early phases of virtmcu, peripherals like UARTs and Radios were modeled with an **Immediate Execution Model**. When the guest firmware executed a store instruction to a peripheral's TX register, the peripheral immediately packed the data and dispatched it over the Zenoh bus. 

Because virtmcu synchronizes virtual time across nodes, this created a critical physical impossibility: a "virtual time flood". The CPU could write 100 bytes to a UART in 100 instructions (virtually ~100 nanoseconds), and all 100 bytes would arrive at the receiving node with identical virtual timestamps, completely bypassing the physical reality of a 115200 baud rate (which should take ~8.6 milliseconds).

To fix this, we must introduce **Peripheral Backpressure** and **Transmission Delay**. This document evaluates the spectrum of how this can be modeled, how other industry tools handle it, and the compromises we are choosing to make.

## 2. Evaluation of Modeling Options

### Option A: Strict Cycle-Accurate & Bus-Contention Modeling
In this model, the emulator simulates the actual clock ticks of the APB/AHB bus. The peripheral asserts wait states, and the CPU pipeline is stalled cycle-by-cycle until the peripheral's state machine shifts the bits out.
*   **Pros:** 100% true to silicon. Catches subtle race conditions in DMA and bus arbitration.
*   **Cons:** Computationally devastating. Requires modeling the entire microarchitecture (pipelines, caches, interconnects). Runs at sub-Megahertz speeds. 

### Option B: SystemC TLM-2.0 Approximately Timed (AT)
Using SystemC, transactions are broken into phases (Begin Request, End Request, Begin Response, End Response). The peripheral explicitly models the delay between receiving a request and fulfilling it, handling backpressure via standard TLM-2.0 semantics.
*   **Pros:** Industry standard for pre-silicon hardware verification.
*   **Cons:** Heavyweight. Forces all peripheral modeling into C++/SystemC, breaking our "Rust-first" memory-safety and concurrency goals for the core simulation loop.

### Option C: Event-Driven Virtual Timers (The Chosen Path)
The peripheral accepts the data into a software FIFO instantly, but schedules a native emulator timer (`QEMUTimer` tied to `QEMU_CLOCK_VIRTUAL`) to fire after the calculated transmission duration (e.g., $10 \text{ bits} / 115200 \text{ bps} = 86.8 \mu s$). When the timer fires, the peripheral triggers the TX empty interrupt.
*   **Pros:** Fast, lightweight, and leverages existing QEMU infrastructure. Easily implemented in Rust (`virtmcu-qom`). Completely solves the "virtual time flood" for the receiving node.
*   **Cons:** Compromises on micro-architectural accuracy (the CPU bus transaction completes instantly, even if the logical data transfer takes time).

### Option D: The Immediate Model (Current State)
Do nothing. Let the CPU blast data instantly.
*   **Pros:** Easiest to write.
*   **Cons:** Useless for testing real-world firmware constraints, network congestion, or receiver buffer management.

## 3. Industry Context: How Others Do It

We evaluated 7 leading simulation and emulation tools to contextualize our decision:

1.  **Upstream QEMU**: Uses a mix of Option C and Option D. Many simple peripherals use immediate execution. However, high-quality models (like `hw/char/cadence_uart.c`) use `QEMUTimer` to simulate baud rates and throttle TX/RX interrupts. *Virtmcu is formally standardizing QEMU's best-practice (Option C) across our entire distributed federation.*
2.  **Renode (Antmicro)**: Primarily uses an event-driven framework (Option C). Renode executes instructions and schedules events in a timeline. Peripherals often accept data instantly and use scheduled events to raise interrupts later. It is instruction-accurate, not cycle-accurate.
3.  **gem5**: Implements Option A. It is highly cycle-accurate, modeling the pipeline, cache hierarchy, and bus matrices. It is preferred by computer architects but is notoriously too slow for interactive embedded software development.
4.  **ARM Fast Models**: Uses Option B (SystemC TLM-2.0 Loosely Timed). It prioritizes software execution speed (instruction-accurate) but allows for timing annotations to approximate peripheral delays.
5.  **Verilator**: Option A (extreme). Compiles RTL (Verilog) to C++. It evaluates the exact boolean logic of the silicon. Perfect accuracy, but runs magnitudes slower than QEMU.
6.  **Qbox / MINRES libqemu**: Wraps QEMU inside SystemC. Relies on SystemC for peripheral timing (Option B/C hybrid). We rejected this because of the severe IPC/locking overhead required to bridge QEMU's execution loop into a SystemC scheduler.
7.  **Wokwi**: A web-based emulator for makers (AVR, ESP32). Uses a custom event-loop (Option C). It approximates cycles and delays enough to make Arduino sketches work with virtual LEDs, but does not guarantee true architectural cycle fidelity.

## 4. Compromising Authenticity: The Worst-Case Scenarios

By choosing **Option C (Event-Driven Virtual Timers)** running on top of QEMU's TCG (Tiny Code Generator), we are explicitly choosing **Instruction-Accuracy over Cycle-Accuracy**. 

We must be radically honest about the modeling compromises this entails:

*   **Compromise 1: 1 Instruction $\neq$ 1 Cycle.** QEMU's `icount` mode arbitrarily assigns a fixed virtual time cost to instructions (e.g., 1 instruction = 1 nanosecond). In reality, a Cortex-M7 floating-point instruction takes different cycles than a simple ALU addition, and pipeline stalls/branch mispredictions cost time. 
*   **Compromise 2: Invisible Bus Contention.** If the CPU and a DMA controller access the same memory bank, physical silicon enforces arbitration wait-states. In virtmcu, memory accesses are serialized by the emulator thread and complete instantly in virtual time. Contention is invisible.
*   **Compromise 3: The Jitter of `slaved-suspend`.** In our high-performance `slaved-suspend` mode, the CPU executes a whole block of instructions (e.g., 1ms worth) before virtual time catches up to wall-clock time. A timer might technically fire at $T=500\mu s$, but the CPU might not process the interrupt until the end of the current translation block.

### Worst-Case Scenario Example:
A developer writes firmware that relies on exact cycle-counting to bit-bang a strict protocol (like WS2812b NeoPixels) without using hardware timers, and tightly interleaves this with DMA transfers. 
*   *What happens?* The firmware will likely fail on physical silicon despite passing in virtmcu, because virtmcu hid the DMA bus contention and incorrectly assumed every bit-bang loop iteration took exactly 3 nanoseconds.

## 5. Impact on Our Target Audience (Embedded Software Developers)

How big of a problem are these compromises? For our primary audience—embedded software engineers writing drivers, network stacks, RTOS tasks, and control logic—**these compromises are entirely acceptable, and often preferred.**

*   **Software Observable Fidelity is the Goal:** Embedded developers rarely care that a cache miss cost 3 extra clock cycles. They care deeply that if they configure a UART to 9600 baud, the `TX_EMPTY` interrupt fires 1 millisecond later, allowing the RTOS to context-switch to another thread in the meantime. 
*   **Finding Real Bugs:** The "virtual time flood" (Option D) hides real software bugs. If a driver assumes a UART is infinitely fast and doesn't handle TX buffer overflow correctly, Option D lets that bug slip into production. Option C triggers the bug in the emulator because the firmware outpaces the simulated baud rate.
*   **Speed is King:** If an automated test suite takes 4 hours in gem5 (Option A) but 2 minutes in virtmcu (Option C), the software team will choose virtmcu. Continuous Integration (CI) requires speed.

## 6. Conclusion
The emulator is not physics. It is a highly optimized functional representation. By utilizing QEMU's `QEMUTimer` to simulate peripheral processing delays (Phase 29), we achieve the "sweet spot": we provide software developers with realistic hardware behavior (backpressure, interrupts, asynchronous completion) while maintaining the execution speed required for modern CI/CD workflows. We accept the loss of microscopic bus-contention accuracy as a necessary trade-off for macro-scale distributed system simulation.
