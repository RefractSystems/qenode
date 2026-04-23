#[repr(C, align(8))]
/// A struct
pub struct QemuMutex {
    _opaque: [u8; 64],
}

#[repr(C, align(8))]
/// A struct
pub struct QemuCond {
    _opaque: [u8; 56],
}

#[cfg(not(test))]
extern "C" {
    /// A function
    pub fn virtmcu_bql_locked() -> bool;
    /// A function
    pub fn virtmcu_bql_lock();
    /// A function
    pub fn virtmcu_bql_unlock();
    /// A function
    pub fn virtmcu_bql_force_unlock();
    /// A function
    pub fn virtmcu_bql_force_lock();

    /// A function
    pub fn virtmcu_mutex_new() -> *mut QemuMutex;
    /// A function
    pub fn virtmcu_mutex_free(mutex: *mut QemuMutex);
    /// A function
    pub fn qemu_mutex_init(mutex: *mut QemuMutex);
    /// A function
    pub fn qemu_mutex_destroy(mutex: *mut QemuMutex);
    /// A function
    pub fn virtmcu_mutex_lock(mutex: *mut QemuMutex);
    /// A function
    pub fn virtmcu_mutex_unlock(mutex: *mut QemuMutex);

    /// A function
    pub fn virtmcu_cond_new() -> *mut QemuCond;
    /// A function
    pub fn virtmcu_cond_free(cond: *mut QemuCond);
    /// A function
    pub fn qemu_cond_init(cond: *mut QemuCond);
    /// A function
    pub fn qemu_cond_destroy(cond: *mut QemuCond);
    /// A function
    pub fn virtmcu_cond_wait(cond: *mut QemuCond, mutex: *mut QemuMutex);
    // Returns non-zero (true) on signal/spurious-wakeup, 0 (false) on timeout.
    // Mirrors QEMU's qemu_cond_timedwait which returns `err != ETIMEDOUT`.
    /// A function
    pub fn virtmcu_cond_timedwait(cond: *mut QemuCond, mutex: *mut QemuMutex, ms: u32) -> i32;
    /// A function
    pub fn virtmcu_cond_signal(cond: *mut QemuCond);
    /// A function
    pub fn virtmcu_cond_broadcast(cond: *mut QemuCond);
}

#[cfg(test)]
mod mock {
    use super::*;
    use std::sync::Mutex;

    static BQL_LOCKED: Mutex<bool> = Mutex::new(false);

    pub fn virtmcu_bql_locked() -> bool {
        *BQL_LOCKED.lock().unwrap()
    }

    pub fn virtmcu_bql_lock() {
        let mut locked = BQL_LOCKED.lock().unwrap();
        *locked = true;
    }

    pub fn virtmcu_bql_unlock() {
        let mut locked = BQL_LOCKED.lock().unwrap();
        *locked = false;
    }

    pub fn virtmcu_bql_force_unlock() {
        virtmcu_bql_unlock();
    }

    pub fn virtmcu_bql_force_lock() {
        virtmcu_bql_lock();
    }

    pub fn virtmcu_mutex_lock(_mutex: *mut QemuMutex) {}
    pub fn virtmcu_mutex_unlock(_mutex: *mut QemuMutex) {}
    pub fn virtmcu_cond_wait(_cond: *mut QemuCond, _mutex: *mut QemuMutex) {}
    pub fn virtmcu_cond_timedwait(_cond: *mut QemuCond, _mutex: *mut QemuMutex, _ms: u32) -> i32 {
        1
    }
    pub fn virtmcu_cond_signal(_cond: *mut QemuCond) {}
    pub fn virtmcu_cond_broadcast(_cond: *mut QemuCond) {}
}

/// A safe wrapper for the Big QEMU Lock (BQL).
pub struct Bql;

impl Bql {
    /// Acquires the BQL and returns a guard. The lock is released when the guard is dropped.
    pub fn lock() -> BqlGuard {
        #[cfg(not(test))]
        unsafe {
            virtmcu_bql_lock();
        };
        #[cfg(test)]
        mock::virtmcu_bql_lock();
        BqlGuard
    }

    /// Acquires the BQL but does NOT return a guard. The lock will remain held.
    /// This is used when transferring lock ownership to a C caller.
    pub fn lock_forget() {
        #[cfg(not(test))]
        unsafe {
            virtmcu_bql_lock();
        };
        #[cfg(test)]
        mock::virtmcu_bql_lock();
    }

    /// Explicitly unlocks the BQL. Use this only when you need to block without holding the lock.
    ///
    /// # Safety
    /// The caller must ensure the BQL is currently held.
    pub unsafe fn unlock() {
        #[cfg(not(test))]
        virtmcu_bql_unlock();
        #[cfg(test)]
        mock::virtmcu_bql_unlock();
    }

    /// Temporarily unlocks the BQL and returns a guard that will relock it when dropped.
    /// Returns None if the BQL was not held.
    pub fn temporary_unlock() -> Option<BqlUnlockGuard> {
        #[cfg(not(test))]
        let was_locked = unsafe { virtmcu_bql_locked() };
        #[cfg(test)]
        let was_locked = mock::virtmcu_bql_locked();

        if was_locked {
            #[cfg(not(test))]
            unsafe {
                virtmcu_bql_force_unlock();
            }
            #[cfg(test)]
            mock::virtmcu_bql_force_unlock();
            Some(BqlUnlockGuard)
        } else {
            None
        }
    }
}

/// A struct
pub struct BqlGuard;

impl Drop for BqlGuard {
    fn drop(&mut self) {
        #[cfg(not(test))]
        unsafe {
            virtmcu_bql_unlock();
        };
        #[cfg(test)]
        mock::virtmcu_bql_unlock();
    }
}

/// A struct
pub struct BqlUnlockGuard;

impl Drop for BqlUnlockGuard {
    fn drop(&mut self) {
        #[cfg(not(test))]
        unsafe {
            virtmcu_bql_force_lock();
        };
        #[cfg(test)]
        mock::virtmcu_bql_force_lock();
    }
}

/// A struct
pub struct QemuMutexGuard<'a> {
    mutex: *mut QemuMutex,
    _marker: core::marker::PhantomData<&'a mut QemuMutex>,
}

impl QemuMutex {
    /// A method
    pub fn lock(&mut self) -> QemuMutexGuard<'_> {
        #[cfg(not(test))]
        unsafe {
            virtmcu_mutex_lock(core::ptr::from_mut(self));
        };
        #[cfg(test)]
        mock::virtmcu_mutex_lock(core::ptr::from_mut(self));
        QemuMutexGuard { mutex: core::ptr::from_mut(self), _marker: core::marker::PhantomData }
    }
}

impl Drop for QemuMutexGuard<'_> {
    fn drop(&mut self) {
        #[cfg(not(test))]
        unsafe {
            virtmcu_mutex_unlock(self.mutex);
        };
        #[cfg(test)]
        mock::virtmcu_mutex_unlock(self.mutex);
    }
}

impl QemuCond {
    /// A method
    pub fn wait(&mut self, mutex: &mut QemuMutex) {
        #[cfg(not(test))]
        unsafe {
            virtmcu_cond_wait(core::ptr::from_mut(self), core::ptr::from_mut(mutex));
        };
        #[cfg(test)]
        mock::virtmcu_cond_wait(core::ptr::from_mut(self), core::ptr::from_mut(mutex));
    }

    /// A method
    pub fn wait_timeout(&mut self, mutex: &mut QemuMutex, ms: u32) -> bool {
        #[cfg(not(test))]
        unsafe {
            virtmcu_cond_timedwait(core::ptr::from_mut(self), core::ptr::from_mut(mutex), ms) != 0
        }
        #[cfg(test)]
        {
            mock::virtmcu_cond_timedwait(core::ptr::from_mut(self), core::ptr::from_mut(mutex), ms)
                != 0
        }
    }

    /// A method
    pub fn signal(&mut self) {
        #[cfg(not(test))]
        unsafe {
            virtmcu_cond_signal(core::ptr::from_mut(self));
        };
        #[cfg(test)]
        mock::virtmcu_cond_signal(core::ptr::from_mut(self));
    }

    /// A method
    pub fn broadcast(&mut self) {
        #[cfg(not(test))]
        unsafe {
            virtmcu_cond_broadcast(core::ptr::from_mut(self));
        };
        #[cfg(test)]
        mock::virtmcu_cond_broadcast(core::ptr::from_mut(self));
    }
}

const _: () = assert!(core::mem::size_of::<QemuMutex>() == 64);
const _: () = assert!(core::mem::size_of::<QemuCond>() == 56);
