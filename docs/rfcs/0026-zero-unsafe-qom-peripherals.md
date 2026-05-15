# RFC-0026: Zero Unsafe QOM Peripherals

## Summary
This RFC proposes architectural enhancements to the `virtmcu_qom` crate to completely eliminate the need for `unsafe` blocks in Rust-based peripheral implementations. It introduces safe abstractions for QOM string properties, safe dependency injection for QOM links, and idiomatic Rust closures for callback contexts. This effectively pushes all C-FFI boundary `unsafe` code down into the framework.

## Motivation
According to RFC-0023 Phase 4, the goal is "Zero Unsafe Boilerplate". Currently, Rust peripherals still require `unsafe` code to:
1. Calculate the length and parse QOM string properties (`*mut c_char`).
2. Dereference parent device pointers to access QOM links during `realize()` and `write()`.
3. Cast opaque pointers (`*mut c_void`) in callback closures (e.g., within `DeterministicReceiver`).

Removing these remaining `unsafe` blocks will ensure peripheral developers cannot accidentally trigger undefined behavior, will improve code readability, and strictly aligns with the project's Enterprise-Ready Quality mandate. Peripherals should focus on domain logic, not FFI memory layout.

## Guide-level explanation
As a peripheral developer, you should never have to type the `unsafe` keyword or deal with C-pointers.

**QOM Properties**
Instead of defining a property as a `*mut c_char` and writing a `while` loop to find the null terminator, you define it using a framework-provided wrapper (e.g., `QomString`). You can then safely call `.as_string()` to get an allocated Rust `String` or `.as_str()` for a borrowed slice.

**Dependency Injection (QOM Links)**
Currently, peripherals store a raw pointer to the parent QEMU object to fetch `QomLink` dependencies like `DataTransport`. With this RFC, the `#[qom_state]` macro and `PeripheralState` initialization will be updated so that by the time `realize()` is called, any declared `#[qom_link]` dependencies are already safely resolved and injected into your Rust state (e.g., as an `Option<Arc<dyn DataTransport>>`).

**Safe Callbacks**
When setting up event listeners, such as a `DeterministicReceiver` for incoming transport packets, you will no longer pass an `opaque` pointer. Instead, the API will accept a standard Rust closure (`impl FnMut(...)`). You can safely use Rust's native `Arc` and `move` semantics to capture your peripheral's shared state inside the closure.

## Reference-level explanation
This proposal involves three primary modifications to `virtmcu_qom` and related APIs:

1. **Safe QOM Properties (`QomString`)**:
   Introduce a `virtmcu_qom::property::QomString` type that wraps the underlying `*mut c_char`. The `#[qom_property]` macro will map string definitions to this type. `QomString` will implement safe accessor methods that internally use `unsafe { core::slice::from_raw_parts }` combined with `std::ffi::CStr` to guarantee safe UTF-8 boundary checks before returning data to the peripheral.

2. **Safe Parent Access & QOM Links**:
   Currently, `PeripheralState::new(qemu_dev: &Self::QomType)` is called during QEMU's `instance_init`. QOM Links, however, are typically populated just before `realize`. The framework must be updated to either:
   - Provide a safe `DeviceHandle<'a, T>` that allows peripherals to safely query the parent without raw pointers.
   - Automatically inject resolved dependencies into the `RustDummyState` struct via the macro layer right before invoking the peripheral's `realize()` method, effectively decoupling the state from the `qemu_dev_ptr`.

3. **Safe Callbacks (Trampoline Pattern)**:
   Update `DeterministicReceiver::new` to accept an `impl FnMut(Packet)` instead of an `opaque: *mut c_void` and a function pointer. Under the hood, `DeterministicReceiver` will `Box` the closure and use a C-compatible trampoline function. The C-API receives the boxed closure's pointer as its `opaque` context, and the trampoline safely casts it back to `&mut dyn FnMut(Packet)` before invoking it. This completely hides the `*mut c_void` cast from the peripheral logic.

## Drawbacks
- Modifying the procedural macros and `PeripheralState` trait may require migrating existing peripherals simultaneously to avoid breaking the build.
- Using a boxed closure for callbacks introduces a slight heap allocation overhead during initialization, though this is a one-time cost and execution overhead is negligible in the context of transport receivers.

## Rationale and alternatives
This design centralizes the FFI complexity inside `virtmcu_qom`, fulfilling the framework's responsibility to shield domain logic from the underlying C execution environment.
An alternative is keeping the `unsafe` code but wrapping it in helper macros within the peripheral code itself. However, that still leaks FFI concerns into the business logic layer and violates the strict encapsulation desired for peripheral models.

## Prior art
Projects like `vhost-user-backend` and Rust's standard library (`std::thread::spawn`) use the trampoline pattern to safely pass Rust closures across C boundaries.

## Unresolved questions
- How does the framework guarantee the parent device's QOM links are fully resolved before `PeripheralState::realize` is called, given QEMU's two-phase initialization (`instance_init` vs `realize`)? We need to verify exactly when the QOM link pointers become valid in the QEMU lifecycle.
- Should `QomString` borrow the string (`&str`) with a lifetime tied to the device, or always return an owned `String`? If borrowed, we must ensure QEMU does not mutate or free the string property unexpectedly.