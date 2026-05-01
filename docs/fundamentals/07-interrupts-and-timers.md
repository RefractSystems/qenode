# Chapter 7: Interrupt Architecture and Temporal Synchronization

## 7.1 The Paradigm of Asynchronous Notification
In early computing architectures, the central processor relied exclusively on "polling"—continuously reading a peripheral's status register in a tight loop to ascertain whether a hardware event (such as a buffer filling or a timer expiring) had occurred. This paradigm is profoundly inefficient; it monopolizes the CPU's execution cycles, precluding the processor from performing useful computation while waiting for high-latency external devices.

To resolve this computational bottleneck, hardware engineers introduced the **Interrupt**. An interrupt is a physical or logical signal indicating to the processor that an event requires immediate attention. It is the architectural embodiment of asynchronous notification. Upon receiving an interrupt, the processor suspends its current execution context (saving the stack and program counter) and vectors execution to a predefined Interrupt Service Routine (ISR). 

## 7.2 The Role of the Interrupt Controller
In a modern System-on-Chip (SoC), it is impractical for every peripheral to possess a dedicated interrupt pin directly connected to the CPU core. Instead, interrupts are routed through a centralized aggregator known as the **Interrupt Controller**.

In ARM Cortex-M profiles, this component is the **Nested Vectored Interrupt Controller (NVIC)**. High-performance application processors (such as the Cortex-A series) employ the **Generic Interrupt Controller (GIC)**. These controllers do not merely multiplex signals; they manage complex prioritization hierarchies, allowing critical hardware faults to preempt the execution of lower-priority peripheral handlers.

In the VirtMCU emulation ecosystem, the Device Tree specifies this topological wiring. A peripheral's `interrupts = <N>` property instructs QEMU to logically bind the peripheral's output IRQ line to input port `N` of the instantiated NVIC or GIC object.

## 7.3 Interrupt Assertion in the Emulator
Within a peripheral plugin (developed in Rust or C), hardware state transitions must manually assert or de-assert these virtual interrupt lines. QEMU exposes this capability through the `qemu_set_irq` API.

```rust
// Emulating a level-triggered hardware interrupt
// Assert the interrupt line (driving the virtual wire high)
qemu_set_irq(dev.irq, 1);

// ... Later, when the guest firmware acknowledges the interrupt ...
// De-assert the interrupt line (driving the virtual wire low)
qemu_set_irq(dev.irq, 0);
```

Failing to properly de-assert a level-triggered interrupt after the guest firmware has serviced it will result in an "interrupt storm"—an infinite loop where the virtual CPU continuously re-enters the ISR, halting all other application logic.

## 7.4 Timers, Virtual Time, and the WFI Trap
Timers are specialized peripherals designed to generate interrupts at configurable intervals. However, in an emulation environment, the concept of "time" bifurcates into two distinct domains: **Wall-Clock Time** (the objective passage of time in the physical universe) and **Virtual Time** (the synthetic passage of time perceived by the emulated firmware).

The `slaved-suspend` and `slaved-icount` modes of VirtMCU mandate that the emulator's internal clocks derive their progression exclusively from Virtual Time. If the simulation is paused waiting for an external network packet, Virtual Time ceases to advance. Consequently, the emulated hardware timers also halt, ensuring that the firmware's temporal logic remains perfectly synchronized with the external simulation world.

### 7.4.1 The Wait For Interrupt (WFI) Instruction
The `WFI` instruction is an architectural directive that instructs the CPU to suspend execution and enter a low-power sleep state until an interrupt is received. In physical silicon, this is a power-saving measure. 

In VirtMCU, `WFI` serves a far more critical role: it enables **Temporal Warping**. If the virtual CPU is asleep and the only pending hardware event is a timer scheduled to expire in 500,000 virtual nanoseconds, the emulation engine does not need to execute 500,000 "no-operation" cycles. It can instantaneously advance the Virtual Time clock by the required delta and immediately fire the timer interrupt, drastically accelerating simulation throughput.

## 7.5 Summary
Interrupts are the primary mechanism for decoupling CPU execution from peripheral latency, orchestrated by the NVIC or GIC. In VirtMCU, precise modeling of virtual time and proper handling of the `WFI` instruction are essential for maintaining both temporal accuracy and high-performance simulation execution.

## 7.6 Exercises
1.  **Level-Triggered vs. Edge-Triggered:** Analyze the theoretical difference between a level-triggered interrupt and an edge-triggered interrupt. How does the QEMU `qemu_set_irq` API map to these two distinct hardware behaviors, and what specific bug is introduced if firmware assumes an interrupt is edge-triggered when the emulator models it as level-triggered?
2.  **The Polling Deadlock:** VirtMCU architecture policies explicitly ban "tight polling loops" in firmware when running in multi-node simulation mode. Drawing upon your understanding of Virtual Time and the Big QEMU Lock (BQL), explain why a tight polling loop (`while(!flag) {}`) that waits for an external network packet will permanently deadlock a deterministic simulation.
