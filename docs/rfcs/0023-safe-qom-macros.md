# RFC 0023: Safe QOM Macros and Boilerplate Eradication (Revised)

## Status
Accepted (implemented in `virtmcu-qom`; every peripheral uses `#[qom_device]`)

## Context & Problem Statement
The boundary between QEMU's C-based Object Model (QOM) and VirtMCU's Rust peripherals is currently a source of critical architectural risk. 

1. **Safety:** Developers must manually manage `Box::into_raw` and `Box::from_raw`, leading to potential memory leaks or Use-After-Free errors if lifecycle hooks are fumbled.
2. **Boilerplate:** Peripherals require extensive "C written in Rust" code (manual `Property` arrays, FFI shims, `TypeInfo` definitions).
3. **DI Hacks:** Dependency Injection of traits (like `DataTransport`) relies on dangerous pointer truncation/casting across QOM links.
4. **Maintenance:** The current bespoke macros are diverging from upstream QEMU's official Rust efforts, creating future technical debt.

## Proposed "Revised SOTA" Architecture
We move away from "black box magic" toward a **Hybrid Macro Approach**. The goal is **"Zero Unsafe Boilerplate"** while maintaining 100% transparency for debugging and synchronization.

### 1. Syntax Mirroring (Upstream Alignment)
We will design our macros to mirror the impending official `qemu/rust` API. This ensures that moving to upstream QEMU in the future is a simple namespace migration rather than a structural refactor.

### 2. Type-State Synchronization (Explicit Safety)
Instead of hiding the `VcpuDrain` lock, we make it visible in the type system. The generated FFI shim will acquire the lock and pass an explicit `DrainToken` to the safe Rust methods. This prevents deadlocks and enforces explicit reasoning about simulation state.

### 3. Debugging Transparency (Explicit Traits)
Macros will generate implementations of safe Rust traits (`Peripheral`, `QomLifecycle`) rather than opaque, hidden functions. This allows GDB/LLDB to point directly to safe Rust code during crashes.

```rust
use virtmcu_qom::macros::{qom_device, qom_property, qom_link};
use virtmcu_qom::device::{MmioResult, DrainToken};

#[qom_device(name = "reference-peripheral", parent = "sys-bus-device")]
pub struct ReferencePeripheral {
    #[qom_property(default = "u64::MAX")]
    pub base_addr: u64,

    #[qom_link(target = "virtmcu-transport-hub")]
    pub transport: QomLink<dyn DataTransport>,

    // Framework manages the state lifecycle and BqlGuarded accessibility
    #[qom_state]
    pub state: ReferencePeripheralState,
}

impl virtmcu_qom::Peripheral for ReferencePeripheral {
    fn realize(&mut self) -> Result<()> {
        let transport = self.transport.get().ok_or("Transport link missing")?;
        self.state.receiver = DeterministicReceiver::new(transport, ...);
        Ok(())
    }

    // Explicit DrainToken prevents accidental lock omission or nested deadlocks
    fn read(&self, addr: u64, size: u32, token: &DrainToken) -> MmioResult<'_> {
        match addr {
            REG_STATUS => MmioResult::wait_for(
                || self.state.has_data.load(),
                || 1,
                || 0
            ),
            _ => MmioResult::Ready(0)
        }
    }
}

// Explicit registration required for DSO (.so) compatibility
virtmcu_qom::register_peripheral!(ReferencePeripheral);
```



## Consequences
*   **Positive:** Guaranteed memory safety at the QOM/Rust boundary.
*   **Positive:** Minimal technical debt vs. upstream QEMU.
*   **Positive:** Retains full system transparency for systems-level debugging.
*   **Negative:** Developers must still understand the concept of "Tokens" for synchronization, though this is considered a standard Rust safety pattern.
