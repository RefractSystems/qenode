# RFC-0027: CoSimBridge RAII IoC Framework

## Status
Accepted

## Context
Various VirtMCU bridging components (such as `mmio-socket-bridge`, `remote-port`, and soon `netdev` and `chardev`) require complex synchronization with QEMU's execution loop. Specifically, they must handle vCPU registration, safely yield the Big QEMU Lock (BQL) when blocked on external I/O, and ensure proper thread draining and teardown when the peripheral is destroyed. Initially, this boilerplate was duplicated across each bridge, violating the DRY principle and introducing subtle race conditions.

## Decision
We introduce the `CoSimBridge` and the `CoSimTransport` trait inside `virtmcu-qom`. `CoSimBridge` acts as an Inversion of Control (IoC) container that abstracts all QEMU threading, BQL yielding, and RAII teardown logic.

**Usage Mandate:** `CoSimBridge` is explicitly designated for **Boundary/Infrastructure** peripherals that talk to external processes without a concept of virtual time (e.g., test runners, HiL hardware, the coordinator itself). It MUST NOT be used for **Simulation** peripherals (e.g., actuators, sensors, SPI, radios) that participate in the PDES graph; those MUST use `DeterministicReceiver` and `reserve()/commit()` to ensure data flows through the `DeterministicCoordinator` and respects the quantum barrier.

## Reference-level explanation
- **`CoSimTransport` Trait**: Implementers define how to read/write data from their specific transport (e.g., sockets, remote-port).
- **`CoSimBridge<T>`**: Takes ownership of a `CoSimTransport`. It exposes a safe API to the peripheral's `read` and `write` MMIO handlers. Under the hood, it automatically drops the BQL using `Bql::temporary_unlock()` when waiting for data, and handles the `VcpuDrain` on `Drop`.

## Consequences
- **Positive**: Eliminates duplicated synchronization logic. "Safety-by-Construction" guarantees that teardowns will not leave dangling pointers or deadlocked threads.
- **Negative**: Increases the abstraction layer inside `virtmcu-qom`, requiring new bridge developers to conform strictly to the `CoSimTransport` trait.