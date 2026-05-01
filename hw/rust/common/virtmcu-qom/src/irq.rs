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

    /// A setter
    #[link_name = "virtmcu_set_irq_hook"]
    fn qemu_virtmcu_set_irq_hook(
        cb: Option<
            extern "C" fn(
                opaque: *mut core::ffi::c_void,
                n: core::ffi::c_int,
                level: core::ffi::c_int,
            ),
        >,
    );
}

use std::sync::Mutex;
static IRQ_HOOKS: Mutex<
    Vec<
        extern "C" fn(opaque: *mut core::ffi::c_void, n: core::ffi::c_int, level: core::ffi::c_int),
    >,
> = Mutex::new(Vec::new());

/// Register a new IRQ hook.
pub fn virtmcu_set_irq_hook(
    cb: Option<
        extern "C" fn(opaque: *mut core::ffi::c_void, n: core::ffi::c_int, level: core::ffi::c_int),
    >,
) {
    let Some(cb) = cb else { return };
    let mut hooks = IRQ_HOOKS.lock().unwrap();
    if hooks.is_empty() {
        unsafe {
            qemu_virtmcu_set_irq_hook(Some(multiplexed_irq_hook));
        }
    }
    hooks.push(cb);
}

extern "C" fn multiplexed_irq_hook(
    opaque: *mut core::ffi::c_void,
    n: core::ffi::c_int,
    level: core::ffi::c_int,
) {
    let hooks = IRQ_HOOKS.lock().unwrap();
    for hook in hooks.iter() {
        hook(opaque, n, level);
    }
}
