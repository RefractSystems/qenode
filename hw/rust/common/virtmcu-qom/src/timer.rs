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
