# RFC-0029: Remote Port (SystemC) Co-Simulation Backbone

## Status
Accepted

## Context
VirtMCU needs to integrate seamlessly with existing EDA and hardware co-simulation tools, most notably SystemC and Verilator. These ecosystems often use the Xilinx Remote Port (libremoteport) protocol for bridging simulation environments.

## Decision
We implement a `remote-port` bridge module (`hw/rust/bridges/remote-port`). This module acts as a QEMU device that translates VirtMCU's internal MMIO accesses into the standard Remote Port protocol over Unix sockets or TCP.

## Reference-level explanation
- **Integration**: Plugs into the `CoSimBridge` container to handle vCPU suspension and RAII teardown cleanly.
- **SystemC Adapter**: We will maintain a `systemc-adapter` tool that implements the other side of the Remote Port protocol, allowing SystemC IP blocks to act as memory-mapped peripherals inside the QEMU guest.

## Consequences
- **Positive**: Standardized interoperability with Xilinx tools, Verilator, and SystemC.
- **Negative**: The Remote Port protocol introduces its own timing and synchronization complexities, which must be carefully aligned with VirtMCU's deterministic clock.