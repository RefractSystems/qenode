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
}

#[cfg(any(test, miri, feature = "standalone", virtmcu_unit_test))]
extern "C" {
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

#[cfg(any(test, miri, feature = "standalone", virtmcu_unit_test))]
use alloc::vec::Vec;

#[cfg(any(test, miri, feature = "standalone", virtmcu_unit_test))]
use std::sync::Mutex;

#[cfg(any(test, miri, feature = "standalone", virtmcu_unit_test))]
static IRQ_HOOKS: Mutex<
    // virtmcu-allow: static_state reasoning="Mock state for local testing"
    Vec<
        extern "C" fn(opaque: *mut core::ffi::c_void, n: core::ffi::c_int, level: core::ffi::c_int),
    >,
> = Mutex::new(Vec::new());

/// Register a new IRQ hook.
pub fn virtmcu_set_irq_hook(
    _cb: Option<
        extern "C" fn(opaque: *mut core::ffi::c_void, n: core::ffi::c_int, level: core::ffi::c_int),
    >,
) {
    #[cfg(any(test, miri, feature = "standalone", virtmcu_unit_test))]
    {
        let Some(cb) = _cb else { return };
        let mut hooks = IRQ_HOOKS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if hooks.is_empty() {
            unsafe {
                qemu_virtmcu_set_irq_hook(Some(multiplexed_irq_hook));
            }
        }
        hooks.push(cb);
    }
}

#[cfg(any(test, miri, feature = "standalone", virtmcu_unit_test))]
extern "C" fn multiplexed_irq_hook(
    opaque: *mut core::ffi::c_void,
    n: core::ffi::c_int,
    level: core::ffi::c_int,
) {
    let hooks = IRQ_HOOKS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    for hook in hooks.iter() {
        hook(opaque, n, level);
    }
}
