#[repr(C)]
/// A struct
pub struct IRQState {
    _opaque: [u8; 0],
}

/// A type alias
pub type QemuIrq = *mut IRQState;

#[derive(Copy, Clone)]
/// A struct
pub struct SafeIrq(pub QemuIrq);
// SAFETY: QemuIrq is a pointer to an IRQState which is managed by QEMU.
// It is safe to send between threads as long as we only use it via QEMU's
// thread-safe APIs (like qemu_set_irq which typically requires BQL or is
// designed for multi-threading).
unsafe impl Send for SafeIrq {}
// SAFETY: See above. SafeIrq is just a wrapper around a raw pointer.
unsafe impl Sync for SafeIrq {}

extern "C" {
    /// A function
    pub fn qemu_set_irq(irq: QemuIrq, level: i32);
    /// A static
    pub static mut virtmcu_irq_hook: Option<
        extern "C" fn(opaque: *mut core::ffi::c_void, n: core::ffi::c_int, level: core::ffi::c_int),
    >;

    /// A setter
    pub fn virtmcu_set_irq_hook(
        cb: Option<
            extern "C" fn(
                opaque: *mut core::ffi::c_void,
                n: core::ffi::c_int,
                level: core::ffi::c_int,
            ),
        >,
    );
}
