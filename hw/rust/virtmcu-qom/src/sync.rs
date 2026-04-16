use core::ffi::c_void;

#[repr(C)]
pub struct QemuMutex {
    _opaque: [u8; 0],
}

#[repr(C)]
pub struct QemuCond {
    _opaque: [u8; 0],
}

extern "C" {
    pub fn bql_lock();
    pub fn bql_unlock();

    pub fn qemu_mutex_init(mutex: *mut QemuMutex);
    pub fn qemu_mutex_destroy(mutex: *mut QemuMutex);
    pub fn qemu_mutex_lock(mutex: *mut QemuMutex);
    pub fn qemu_mutex_unlock(mutex: *mut QemuMutex);

    pub fn qemu_cond_init(cond: *mut QemuCond);
    pub fn qemu_cond_destroy(cond: *mut QemuCond);
    pub fn qemu_cond_wait(cond: *mut QemuCond, mutex: *mut QemuMutex);
    // Returns 0 on success, non-zero on timeout
    pub fn qemu_cond_timedwait(cond: *mut QemuCond, mutex: *mut QemuMutex, ms: u32) -> i32;
    pub fn qemu_cond_signal(cond: *mut QemuCond);
    pub fn qemu_cond_broadcast(cond: *mut QemuCond);
}

/// A safe wrapper for the Big QEMU Lock (BQL).
pub struct Bql;

impl Bql {
    /// Acquires the BQL and returns a guard. The lock is released when the guard is dropped.
    pub fn lock() -> BqlGuard {
        unsafe { bql_lock() };
        BqlGuard
    }

    /// Explicitly unlocks the BQL. Use this only when you need to block without holding the lock.
    /// Safety: The caller must ensure the BQL is currently held.
    pub unsafe fn unlock() {
        bql_unlock();
    }
}

pub struct BqlGuard;

impl Drop for BqlGuard {
    fn drop(&mut self) {
        unsafe { bql_unlock() };
    }
}
