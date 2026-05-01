# Chapter 5: Emulator Internals

## The Execution Engine

VirtMCU uses QEMU (11.0.0) not as a standalone application, but as a high-speed JIT execution worker. By leveraging QEMU's Tiny Code Generator (TCG), VirtMCU can emulate complex ARM and RISC-V cores at near-native speeds while maintaining the ability to intercept every instruction and memory access.

---

## 1. The MMIO Lifecycle: Firmware to Physics

Understanding how an instruction in the guest firmware ultimately results in a physical action in the simulation (or a SystemC transaction) is critical to the VirtMCU theory of operation.

### Step 1: The Guest Instruction (Firmware)
The firmware executes a standard store instruction to a hardware register:
```assembly
LDR R0, =0x40013000  // Base address of a PWM peripheral
LDR R1, =0x0000007F  // Target duty cycle value
STR R1, [R0, #0x04]  // Write to the PWM_DUTY register (offset 0x04)
```
The firmware has no knowledge of the simulator. It expects this write to change physical voltage.

### Step 2: The QEMU TCG Intercept (Emulator)
Because `0x40013000` is mapped as an MMIO region rather than standard RAM, QEMU's software memory management unit (`softmmu`) intercepts the write during TCG execution.

### Step 3: The MemoryRegion Routing (QOM)
QEMU looks up `0x40013000` in its memory tree and finds the `MemoryRegionOps` struct associated with our custom peripheral. It invokes the C-level `write` callback defined in that struct, passing the **relative offset** (`0x04`) and the data (`0x7F`).

### Step 4: The Language Boundary (C to Rust/SystemC)
Execution now branches depending on the peripheral's implementation:

*   **Native Rust Peripherals (`virtmcu-qom`)**: QEMU calls an `extern "C"` trampoline. The trampoline safely casts the raw C `opaque` pointer to the Rust peripheral struct and invokes its `.write(offset, data, size)` trait method.
*   **SystemC/Verilator Models (`mmio-socket-bridge`)**: The write lands in the Rust `mmio-socket-bridge`. The bridge serializes the offset and data into a binary packet and sends it over a UNIX socket to the `systemc_adapter` process. The QEMU vCPU thread **blocks** (safely yielding the BQL via `Bql::temporary_unlock()`) until the SystemC TLM-2.0 transaction completes.

---

## 2. Dynamic Machine Models

VirtMCU eliminates the need to recompile QEMU to define new hardware boards through two core technologies:

### arm-generic-fdt
This patch series allows QEMU to instantiate CPUs, memory, and peripherals entirely from a Flattened Device Tree (FDT) blob at runtime. 
- **Usage**: `-machine arm-generic-fdt -hw-dtb board.dtb`
- **Impact**: The DTB becomes the single source of truth for the hardware layout.

### QOM Plugin Infrastructure
VirtMCU peripherals are compiled as proper QEMU modules (`--enable-modules`). The resulting `.so` files are auto-discovered by QEMU. This allows developers to iterate on a single peripheral model (e.g., a new CAN controller) and reload it into the simulation without rebuilding the entire emulator.

---

## 3. Core Patch Set

To achieve determinism, we apply a minimal set of strategic patches to QEMU:

1.  **TCG Quantum Hook**: AST-injects `VirtMCU_tcg_quantum_hook` into `accel/tcg/cpu-exec.c`. This allows the `clock` device to pause the vCPU at exact virtual time boundaries.
2.  **Zenoh Backends**: Registers native `netdev` and `chardev` backends to route traffic onto the simulation bus.
3.  **BQL Helpers**: Injects thread-safe wrappers for Big QEMU Lock management, allowing plugins to yield the lock during blocking I/O safely.

---

## 4. Common Pitfalls

### SysBus Mapping vs. `-device`
In the `arm-generic-fdt` machine, adding a device via `-device` is necessary but **insufficient**. QEMU will instantiate the object, but it will not automatically map its MMIO regions. Mapping only occurs if a corresponding node exists in the DTB with a `reg` property.

### Relative Offsets
The `mmio-socket-bridge` delivers **offsets relative to the region base**, not absolute physical addresses. Adapters must NOT add the base address back, as QEMU performs the subtraction before invoking the callback.
