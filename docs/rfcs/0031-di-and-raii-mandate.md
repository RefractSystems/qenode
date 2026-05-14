# RFC-0031: Global State, Dependency Injection (DI), and RAII Mandate

## Summary
This RFC formalizes the strict prohibition of global state (singletons) in peripheral development and mandates the use of Dependency Injection (DI) and Resource Acquisition Is Initialization (RAII) for all resource management.

## Motivation
VirtMCU peripherals are compiled as Dynamic Shared Objects (DSOs) and loaded into QEMU via `dlopen`. This environment creates a "DSO Boundary Isolation Trap":
1. **Duplicate Static Variables**: Rust's standard `static` and `static mut` (including `lazy_static!` and `OnceLock`) are often duplicated by the linker when multiple `.so` files are loaded. State meant to be "global" becomes siloed within each plugin, leading to invisible data loss.
2. **Teardown Use-After-Free**: If a plugin is unloaded (`dlclose`) while a background thread is still running, the process will crash. We need a guaranteed way to join threads *before* the code segment is removed.
3. **Test Isolation**: Deterministic testing requires that each test run starts with a clean state. Global variables preserve state across tests, causing non-deterministic "cross-talk" failures.

## Guide-level explanation
To write a safe peripheral in VirtMCU, you must follow these three rules:

1. **No Globals**: Never use `static mut`, `lazy_static!`, or `thread_local!` for simulation state.
2. **Inject Dependencies**: If your peripheral needs to talk to the network or the clock, do not "look it up" via a global registry. The framework will pass a pointer to the `DataTransport` or `Clock` into your `realize` function. Store this in your peripheral struct.
3. **RAII Destructors**: Use Rust's `Drop` trait to manage cleanup. When your peripheral's `State` struct is dropped:
   - Background threads must be signaled to stop and then joined.
   - Zenoh subscriptions must be dropped.
   - Any raw FFI pointers must be freed.

## Reference-level explanation

### The Dependency Injection (DI) Pattern
Peripherals must use the "Hub" pattern. During the QEMU `realize` phase, the peripheral retrieves its dependencies from a central `virtmcu-transport-hub` object.
```rust
// CORRECT: Retrieve transport via DI
let transport_hub = s.transport_hub;
let transport = virtmcu_qom::hub::get_transport(transport_hub);
s.rust_state.transport = Some(transport);
```

### RAII and VcpuDrain
To safely handle the transition between the QEMU C-heap and the Rust heap, the framework mandates the use of `VcpuDrain`. This utility ensures that the peripheral is not destroyed while a vCPU is currently executing an MMIO read/write.
```rust
impl Drop for MyPeripheralState {
    fn drop(&mut self) {
        self.stop_signal.store(true, Ordering::SeqCst);
        // RAII ensures the background thread is joined here
        // VcpuDrain ensures we wait for the CPU to finish its current MMIO access
    }
}
```

## Drawbacks
- **Boilerplate**: Developers must pass objects through constructors and hubs, which is more verbose than a global `GET_CLOCK()` macro.
- **Complexity**: Understanding the interaction between `Arc<T>`, `VcpuDrain`, and `Drop` requires more Rust expertise than standard bare-metal development.

## Rationale and alternatives
- **Alternative: Global Registry**: We could maintain a global hashmap of all devices in the main QEMU binary. However, this creates a bottleneck and requires complex C-to-Rust string lookups for every access, which is slower than following a pointer injected during initialization.
- **Alternative: Explicit `deinit()`**: We could require developers to call an explicit `cleanup()` function. History shows that humans (and AI agents) eventually forget to call it, leading to leaks and segfaults. RAII makes cleanup "mandatory by construction."

## Prior art
- **Rust's Ownership Model**: This RFC simply extends Rust's core philosophy of RAII to the difficult boundary of C-FFI and dynamic linking.
- **Dependency Injection**: Standard practice in SOTA enterprise software (Java Spring, Google Dagger) to ensure testability and modularity.

## Unresolved questions
- How do we handle "singleton" hardware resources (like a global Power Management Controller) where only one instance *should* exist? (Proposed: Enforce via the YAML topology validator rather than code-level globals).