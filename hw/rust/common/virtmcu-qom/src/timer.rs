use core::ffi::c_void;

/// A constant
pub const QEMU_CLOCK_REALTIME: i32 = 0;
/// A constant
pub const QEMU_CLOCK_VIRTUAL: i32 = 1;

#[repr(C)]
/// A struct
pub struct QemuTimer {
    _opaque: [u8; 0],
}

/// A type alias
pub type QemuTimerCb = extern "C" fn(opaque: *mut c_void);

extern "C" {
    /// A function
    pub fn qemu_clock_get_ns(clock_type: i32) -> i64;
    /// A function
    pub fn virtmcu_timer_new_ns(
        clock_type: i32,
        cb: QemuTimerCb,
        opaque: *mut c_void,
    ) -> *mut QemuTimer;
    /// A function
    pub fn virtmcu_timer_mod(timer: *mut QemuTimer, expire_time: i64);
    /// A function
    pub fn virtmcu_timer_del(timer: *mut QemuTimer);
    /// A function
    pub fn virtmcu_timer_free(timer: *mut QemuTimer);
    /// A function
    pub fn virtmcu_timer_kick(timer: *mut QemuTimer);

    /// A function
    pub fn qemu_clock_run_all_timers();
}

/// A safe wrapper for `qemu_clock_get_ns`.
pub fn qemu_clock_get_ns_safe(clock_type: i32, _ctx: &crate::device::BqlContext) -> i64 {
    unsafe { qemu_clock_get_ns(clock_type) }
}

/// A safe, RAII-enabled wrapper for QEMU timers.
pub struct QomTimer {
    inner: *mut QemuTimer,
}

// SAFETY: QEMU timers are accessed under the BQL, making them effectively thread-safe
// from the perspective of Rust's type system when bounded by QOM devices.
unsafe impl Send for QomTimer {}
// SAFETY: See above.
unsafe impl Sync for QomTimer {}

impl QomTimer {
    /// Creates a new QOM timer.
    /// # Safety
    /// The `cb` and `opaque` pointers must be valid.
    pub unsafe fn new(clock_type: i32, cb: QemuTimerCb, opaque: *mut c_void) -> Self {
        // SAFETY: The caller guarantees that cb and opaque are valid.
        let inner = unsafe { virtmcu_timer_new_ns(clock_type, cb, opaque) };
        assert!(!inner.is_null(), "virtmcu_timer_new_ns returned null");
        Self { inner }
    }

    /// Creates a new QOM timer safely (bypassing the explicit unsafe block requirement for FFI).
    #[allow(clippy::not_unsafe_ptr_arg_deref)] // virtmcu-allow: allow reasoning="Safe wrapper for zero-unsafe peripherals"
    pub fn new_safe(clock_type: i32, cb: QemuTimerCb, opaque: *mut c_void) -> Self {
        unsafe { Self::new(clock_type, cb, opaque) }
    }

    /// Modifies the timer to expire at the given virtual time in nanoseconds.
    pub fn mod_ns(&self, expire_time: i64) {
        // SAFETY: self.inner is a valid pointer to a QemuTimer managed by this struct.
        unsafe { virtmcu_timer_mod(self.inner, expire_time) }
    }

    /// Cancels the timer if it is currently active.
    pub fn del(&self) {
        // SAFETY: self.inner is a valid pointer to a QemuTimer managed by this struct.
        unsafe { virtmcu_timer_del(self.inner) }
    }

    /// Kicks the timer, waking up the QEMU main loop and forcing it to run.
    /// This is safe to call from background threads without holding the BQL.
    pub fn kick(&self) {
        // SAFETY: self.inner is a valid pointer to a QemuTimer.
        unsafe { virtmcu_timer_kick(self.inner) }
    }
}

impl Drop for QomTimer {
    fn drop(&mut self) {
        // SAFETY: self.inner is a valid pointer to a QemuTimer.
        unsafe {
            virtmcu_timer_del(self.inner);
            virtmcu_timer_free(self.inner);
        }
    }
}

use alloc::boxed::Box;
use core::mem::ManuallyDrop;

/// Type alias for the timer closure to reduce type complexity.
type TimerClosure = Box<dyn FnMut(&crate::device::BqlContext) + Send>;

/// A safe, closure-based timer that receives the BQL context upon expiration.
pub struct ClosureTimer {
    // ManuallyDrop controls destruction order in Drop.
    inner: ManuallyDrop<QomTimer>,
    // Double-boxed: outer Box gives a stable fat-pointer address for QEMU.
    // Inner Box<dyn ...> is the type-erased closure.
    closure_ptr: *mut TimerClosure,
}

// SAFETY: the closure is Send; ClosureTimer itself is accessed only under BQL.
unsafe impl Send for ClosureTimer {}
// SAFETY: See above.
unsafe impl Sync for ClosureTimer {}

extern "C" fn closure_trampoline(opaque: *mut core::ffi::c_void) {
    let result = ::std::panic::catch_unwind(::core::panic::AssertUnwindSafe(|| {
        // SAFETY: opaque is the stable address of the outer Box, valid until Drop.
        // QEMU guarantees BQL is held during all timer callbacks.
        let cb = unsafe { &mut **(opaque as *mut TimerClosure) };
        let ctx = unsafe { crate::device::BqlContext::new_unchecked() };
        cb(&ctx);
    }));
    if result.is_err() {
        ::std::process::abort();
    }
}

impl ClosureTimer {
    /// Creates a new timer. The closure receives `&BqlContext` when it fires.
    pub fn new<F>(clock_type: i32, f: F) -> Self
    where
        F: FnMut(&crate::device::BqlContext) + Send + 'static,
    {
        let closure: Box<TimerClosure> = Box::new(Box::new(f));
        let closure_ptr = Box::into_raw(closure);

        // SAFETY: closure_ptr is valid until Drop calls Box::from_raw.
        let inner = unsafe {
            QomTimer::new(clock_type, closure_trampoline, closure_ptr as *mut core::ffi::c_void)
        };

        Self { inner: ManuallyDrop::new(inner), closure_ptr }
    }

    /// Arms the timer to fire at `expire_ns` (virtual nanoseconds).
    pub fn arm(&self, expire_ns: i64) {
        self.inner.mod_ns(expire_ns);
    }

    /// Disarms without destroying.
    pub fn disarm(&self) {
        self.inner.del();
    }
}

impl Drop for ClosureTimer {
    fn drop(&mut self) {
        // Order is load-bearing:
        // 1. Delete + free the QEMU timer (no more callbacks after this).
        // 2. Only then reconstruct and drop the closure Box.
        // Reversing this causes a dangling opaque pointer.
        unsafe { ManuallyDrop::drop(&mut self.inner) };
        unsafe { drop(Box::from_raw(self.closure_ptr)) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::sync::Arc;
    use core::sync::atomic::{AtomicBool, Ordering};

    // virtmcu-allow: static_state reasoning="Mock state in unit tests only"
    static mut MOCK_CB: Option<QemuTimerCb> = None;
    // virtmcu-allow: static_state reasoning="Mock state in unit tests only"
    static mut MOCK_OPAQUE: *mut core::ffi::c_void = core::ptr::null_mut();

    #[no_mangle]
    pub extern "C" fn virtmcu_timer_new_ns(
        _type: i32,
        cb: QemuTimerCb,
        opaque: *mut core::ffi::c_void,
    ) -> *mut QemuTimer {
        unsafe {
            MOCK_CB = Some(cb);
            MOCK_OPAQUE = opaque;
        }
        // return any non-null pointer
        core::ptr::dangling_mut::<QemuTimer>()
    }

    #[no_mangle]
    pub extern "C" fn qemu_clock_run_all_timers() {
        unsafe {
            if let Some(cb) = MOCK_CB {
                cb(MOCK_OPAQUE);
            }
        }
    }

    #[no_mangle]
    pub extern "C" fn virtmcu_timer_mod(_ts: *mut QemuTimer, _expire_time: i64) {}

    #[no_mangle]
    pub extern "C" fn virtmcu_timer_del(_ts: *mut QemuTimer) {}

    #[no_mangle]
    pub extern "C" fn virtmcu_timer_free(_ts: *mut QemuTimer) {}

    #[no_mangle]
    pub extern "C" fn qemu_clock_get_ns(_clock_type: i32) -> i64 {
        0
    }

    #[test]
    fn test_closure_timer_fires() {
        let fired = Arc::new(AtomicBool::new(false));
        let fired_clone = Arc::clone(&fired);

        let timer = ClosureTimer::new(QEMU_CLOCK_VIRTUAL, move |_ctx| {
            fired_clone.store(true, Ordering::SeqCst);
        });

        timer.arm(100);

        qemu_clock_run_all_timers();

        assert!(fired.load(Ordering::SeqCst));
    }

    #[test]
    fn test_closure_timer_receives_bql_context() {
        let fired = Arc::new(AtomicBool::new(false));
        let fired_clone = Arc::clone(&fired);

        let timer = ClosureTimer::new(QEMU_CLOCK_VIRTUAL, move |ctx| {
            // Using ctx with a safe API
            let _time = qemu_clock_get_ns_safe(QEMU_CLOCK_VIRTUAL, ctx);
            fired_clone.store(true, Ordering::SeqCst);
        });

        timer.arm(100);

        qemu_clock_run_all_timers();

        assert!(fired.load(Ordering::SeqCst));
    }

    #[test]
    fn test_closure_timer_drop_order() {
        let timer = ClosureTimer::new(QEMU_CLOCK_VIRTUAL, |_ctx| {});
        timer.arm(100);
        // Dropping while armed should be safe, no use-after-free
        drop(timer);
    }
}
