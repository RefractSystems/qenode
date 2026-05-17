# RFC-0026: Zero Unsafe QOM Peripherals

## Summary

This RFC proposes architectural enhancements to the `virtmcu_qom` crate to completely eliminate the need for `unsafe` blocks in Rust-based peripheral implementations. It introduces safe abstractions for QOM string properties, safe dependency injection for QOM links, and idiomatic Rust closures for callback contexts — pushing all C-FFI boundary `unsafe` code down into the framework.

## Motivation

According to RFC-0023 Phase 4, the goal is "Zero Unsafe Boilerplate." Rust peripherals still require `unsafe` code for three reasons:

1. Calculating length and parsing QOM string properties (`*mut c_char`).
2. Dereferencing parent device pointers to access QOM links during `realize()` and `write()`.
3. Casting opaque pointers (`*mut c_void`) in callback closures (e.g., within `VtimeIngress`).

Removing these `unsafe` blocks ensures peripheral developers cannot accidentally trigger undefined behavior, improves code readability, and aligns with the Enterprise-Ready Quality mandate. Peripherals should contain domain logic, not FFI memory layout.

## Detailed Design

This RFC proposes three modifications to `virtmcu_qom` and related APIs:

### 1. Safe QOM Properties (`QomString`)

Introduce `virtmcu_qom::property::QomString` wrapping the underlying `*mut c_char`. The `#[qom_property]` macro maps string definitions to this type. `QomString` implements safe accessor methods that use `std::ffi::CStr` to guarantee safe UTF-8 boundary checks before returning data to the peripheral.

```rust
// Peripheral code — no unsafe required:
let node_id = self.federation_id.as_str()?;
```

`QomString` returns an owned `String` rather than a borrowed `&str`, because QEMU retains ownership of the property buffer and may theoretically mutate it. The allocation is a one-time cost at `realize()` time.

### 2. Safe Parent Access via `QomLink<T>`

`QomLink<dyn DataTransport>` resolves the dependency during `realize` via the `#[qom_device]`-generated dispatch. By the time `realize(&mut self, ctx: &BqlContext)` is called, any declared `QomLink` dependency is already resolved. The peripheral stores the resolved `Arc<dyn DataTransport>` directly in state — no raw parent-pointer dereference in peripheral code.

### 3. Safe Callbacks (Double-Box Trampoline Pattern)

Both `VtimeIngress::new_safe` (closure-based ingress) and `ClosureTimer` (closure-based timer callbacks, RFC-0041) use a double-box trampoline:

- The outer `Box::into_raw` gives QEMU a stable fat-pointer address for the opaque pointer.
- The inner `Box<dyn FnMut(...)>` provides type-erased dispatch.
- `catch_unwind` + `process::abort()` in both trampolines enforces the "Fail Loudly" mandate across `extern "C"` FFI boundaries (panicking through FFI is Undefined Behavior).
- The `ClosureTimer` closure signature is `FnMut(&BqlContext)`, passing compile-time BQL proof directly into the callback body.

## Drawbacks

- Modifying the procedural macros and `PeripheralState` trait may require migrating existing peripherals simultaneously to avoid breaking the build.
- Boxed closures introduce a one-time heap allocation at initialization; execution overhead is negligible.

## Rationale and Alternatives

This design centralizes FFI complexity inside `virtmcu_qom`, fulfilling the framework's responsibility to shield domain logic from the underlying C environment.

**Alternative: helper macros inside peripheral code.** Keeps `unsafe` in scope, leaks FFI concerns into business logic, and violates the encapsulation goal. Rejected.

## Prior Art

`vhost-user-backend` and Rust's standard library (`std::thread::spawn`) use the trampoline pattern to safely pass closures across C boundaries.

## Unresolved Questions

- **QomLink resolution timing**: QOM Links are populated just before `realize` in QEMU's two-phase init. The framework must verify exactly when link pointers become valid and enforce this via the `#[qom_device]` macro dispatch order. *The resolution path via `QomLink::resolve()` inside `realize` is implemented and working; the exact QEMU lifecycle guarantee is documented in `virtmcu-qom/src/qom.rs`.*

## Related

- RFC-0023: Safe QOM Macros and Boilerplate Eradication
- RFC-0031: DI and RAII Mandate
- RFC-0041: Safe QOM Framework Boundaries — completes this RFC with `ClosureTimer` and the `BqlContext` proof token
