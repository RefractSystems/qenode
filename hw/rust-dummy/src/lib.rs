//! rust-dummy — minimal #[no_std] Rust peripheral template for virtmcu.
//!
//! This library is compiled to a static archive (`librust_dummy.a`) by Meson
//! and linked into `hw-virtmcu-rust-dummy.so`.  It exports three C-callable
//! functions that the C wrapper (`rust-dummy.c`) forwards MMIO operations to.
//!
//! # Extending this template
//!
//! To add per-instance Rust state:
//!
//! 1. Define a state struct:
//!    ```rust
//!    #[repr(C)]
//!    pub struct RustPrivate {
//!        counter: u64,
//!    }
//!    ```
//!
//! 2. Export an init function that the C `realize` hook calls once per device
//!    instance.  Return a raw pointer that C stores in `RustDummyState.rust_priv`.
//!    Never expose `Box` directly — use `Box::into_raw` / `Box::from_raw`:
//!    ```rust
//!    #[no_mangle]
//!    pub extern "C" fn rust_dummy_init() -> *mut RustPrivate {
//!        // NOTE: requires an allocator.  For #[no_std] you must either link
//!        // one (e.g. via extern crate alloc) or use a fixed-size static.
//!        Box::into_raw(Box::new(RustPrivate { counter: 0 }))
//!    }
//!    ```
//!
//! 3. Cast `priv_state` back to a typed pointer inside read/write:
//!    ```rust
//!    pub extern "C" fn rust_dummy_read(priv_state: *mut RustPrivate, ...) -> u64 {
//!        let state = unsafe { &mut *priv_state };
//!        state.counter
//!    }
//!    ```
//!
//! # Safety
//!
//! The `priv_state` pointer originates from C (QEMU's device state struct) and
//! is valid for the lifetime of the device.  QEMU serialises MMIO accesses with
//! the BQL (Big QEMU Lock), so concurrent access is not a concern in normal
//! single-CPU mode.  In SMP mode the caller (`mmio-socket-bridge` or
//! `QemuAdapter`) must ensure serialisation before calling these functions.

#![no_std]

use core::ffi::c_void;
use core::panic::PanicInfo;

/// Called by `c_bridge_read` in `rust-dummy.c` for every guest MMIO read.
///
/// # Arguments
/// * `priv_state` — pointer to Rust-owned per-instance state (NULL in this
///   stateless demo; see module docs for how to add state).
/// * `addr`  — byte offset within the mapped MMIO region.
/// * `size`  — access width in bytes (1, 2, 4, or 8).
///
/// # Returns
/// The value to deliver to the guest.
#[no_mangle]
pub extern "C" fn rust_dummy_read(_priv_state: *mut c_void, addr: u64, _size: u32) -> u64 {
    // Demo: reading offset 0 returns a magic sentinel; all other offsets → 0.
    // Replace this with real register logic when building an actual peripheral.
    match addr {
        0 => 0xdead_beef,
        _ => 0,
    }
}

/// Called by `c_bridge_write` in `rust-dummy.c` for every guest MMIO write.
///
/// # Arguments
/// * `priv_state` — pointer to Rust-owned per-instance state (NULL here).
/// * `addr`  — byte offset within the mapped MMIO region.
/// * `val`   — value written by the guest (zero-extended to 64 bits).
/// * `size`  — access width in bytes (1, 2, 4, or 8).
#[no_mangle]
pub extern "C" fn rust_dummy_write(_priv_state: *mut c_void, _addr: u64, _val: u64, _size: u32) {
    // Demo: writes are silently ignored.
    // Replace with register write logic and side-effects as needed.
}

/// Panic handler required by the compiler for #[no_std] crates.
///
/// With `-C panic=abort` (passed by Meson and set in Cargo.toml) the compiler
/// generates an `abort()` call for every `panic!()` invocation — this function
/// is dead code and will never be called.  It is retained because the compiler
/// requires a panic handler symbol to be present even in abort mode.
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    // Safety: abort() is always safe to call and is the correct response to an
    // unrecoverable error inside a C-linked library.  loop{} would hang the
    // vCPU thread indefinitely, which is far worse.
    extern "C" {
        fn abort() -> !;
    }
    unsafe { abort() }
}
