# RFC-0031: Global State, Dependency Injection (DI), and RAII Mandate

## Summary

This RFC prohibits global state (singletons) in peripheral development and mandates Dependency Injection (DI) and RAII for all resource management in the VirtMCU framework.

## Motivation

VirtMCU peripherals are compiled as Dynamic Shared Objects (DSOs) and loaded into QEMU via `dlopen`. This environment creates a "DSO Boundary Isolation Trap" with three concrete failure modes:

1. **Duplicate Static Variables**: Rust's `static` and `static mut` (including `lazy_static!` and `OnceLock`) are often duplicated by the linker when multiple `.so` files are loaded. State intended to be "global" becomes siloed within each plugin, producing invisible data loss.
2. **Teardown Use-After-Free**: If a plugin is unloaded (`dlclose`) while a background thread is still running, the process crashes. We need a guaranteed mechanism to join threads *before* the code segment is removed. Explicit `deinit()` functions are reliably forgotten; RAII `Drop` cannot be forgotten.
3. **Test Isolation**: Deterministic testing requires each test run to start with clean state. Global variables preserve state across tests, causing non-deterministic cross-talk failures.

## Decision

### Prohibition

`static mut`, `lazy_static!`, `OnceLock`, and `thread_local!` are prohibited for simulation state in peripheral crates. Resources shared across the simulation graph (transports, clocks) must arrive via Dependency Injection.

### Dependency Injection Pattern

Dependencies are injected via `QomLink<T>` — the QOM-typed link mechanism resolved during the `realize` phase. By the time `realize(&mut self, ctx: &BqlContext)` is called, any `QomLink<dyn DataTransport>` declared on the state is already resolved.

```rust
// CORRECT: transport injected by framework via QomLink, resolved before realize()
fn realize(&mut self, ctx: &BqlContext) -> Result<(), String> {
    let transport = self.transport_link
        .resolve()
        .expect("FATAL: transport QomLink not resolved — check topology YAML");
    self.transport = Some(transport);
    Ok(())
}
```

State that must be shared across tasks is wrapped in `Arc` and moved via constructor arguments, not fetched from a registry.

### RAII and `VcpuDrain`

Cleanup is mandatory by construction via `Drop`. The `VcpuDrain` utility ensures the peripheral is not destroyed while a vCPU is executing an MMIO handler. Background thread shutdown signals are set in `Drop`; threads are joined before the destructor returns.

Singletons that *should* be unique (e.g., a global Power Management Controller) are enforced by the topology YAML validator — the YAML schema rejects duplicate peripheral instances of the same type, making the constraint declarative rather than code-level.

## Drawbacks

- **Boilerplate**: passing dependencies through constructors and `QomLink` is more verbose than a `GET_TRANSPORT()` macro.
- **Rust expertise required**: understanding `Arc<T>`, `VcpuDrain`, and `Drop` across the C-FFI boundary requires more background than bare-metal development.

## Rationale and Alternatives

- **Global Registry**: a global hashmap of all devices in the main QEMU binary. Creates a bottleneck and requires C-to-Rust string lookups on every access. Rejected.
- **Explicit `deinit()`**: history shows humans and AI agents forget to call it, causing leaks and segfaults. RAII makes cleanup mandatory by construction. Rejected.

## Prior Art

- **Rust's Ownership Model**: this RFC extends Rust's core RAII philosophy to the C-FFI and dynamic-linking boundary.
- **Java Spring / Google Dagger**: DI is standard practice in enterprise software for testability and modularity.

## Unresolved Questions

None. Singleton hardware resources (e.g., a global Power Management Controller) are enforced by the topology YAML validator rather than code-level globals — validated and implemented.

## Related

- RFC-0023: Safe QOM Macros (`QomLink` injection mechanism)
- RFC-0026: Zero Unsafe QOM Peripherals
- RFC-0041: Safe QOM Framework Boundaries (`QomLink` resolved before `realize(&mut self, ctx: &BqlContext)`)
