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

#[cfg(not(any(test, miri)))]
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

#[cfg(any(test, miri))]
mod mock {
    use super::*;
    use alloc::sync::Arc;
    use std::collections::HashMap;
    use std::sync::{Condvar, Mutex};
    use std::thread_local;

    // Thread-local BQL flag: each test thread has isolated BQL state, preventing
    // interference when cargo test runs test functions in parallel.
    thread_local! {
        static BQL_HELD: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
    }

    // Per-condvar state: a generation counter incremented on every signal/broadcast.
    // Waiters snapshot the generation on entry; a changed generation means "signaled".
    // This approach is immune to lost wakeups (signal before wait) and stale entries
    // from previous tests that reused the same stack address.
    /// A registry of condition variables and mutexes for mocks.
    static MOCK_REGISTRY: Mutex<Option<HashMap<usize, Arc<MockState>>>> = Mutex::new(None);

    pub struct MockState {
        pub cv: Condvar,
        pub gen: Mutex<u64>,
        pub mutex: Mutex<bool>,
        pub waiter_count: Mutex<usize>,
    }

    pub fn get_or_create_mock(addr: usize) -> Arc<MockState> {
        let mut guard = MOCK_REGISTRY.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let map = guard.get_or_insert_with(HashMap::new);
        Arc::clone(map.entry(addr).or_insert_with(|| {
            Arc::new(MockState {
                cv: Condvar::new(),
                gen: Mutex::new(0u64),
                mutex: Mutex::new(false),
                waiter_count: Mutex::new(0),
            })
        }))
    }

    pub fn virtmcu_bql_lock() {
        BQL_HELD.with(|b| b.set(true));
    }

    pub fn virtmcu_bql_unlock() {
        BQL_HELD.with(|b| b.set(false));
    }

    pub fn virtmcu_bql_force_unlock() {
        virtmcu_bql_unlock();
    }

    pub fn virtmcu_bql_force_lock() {
        virtmcu_bql_lock();
    }

    pub fn virtmcu_bql_locked() -> bool {
        BQL_HELD.with(core::cell::Cell::get)
    }

    pub fn virtmcu_mutex_lock(mutex: *mut QemuMutex) {
        let state = get_or_create_mock(mutex as usize);
        let mut locked = state.mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        while *locked {
            locked = state.cv.wait(locked).unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        *locked = true;
    }

    pub fn virtmcu_mutex_unlock(mutex: *mut QemuMutex) {
        let state = get_or_create_mock(mutex as usize);
        let mut locked = state.mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        *locked = false;
        state.cv.notify_all();
    }

    pub fn virtmcu_cond_signal(cond: *mut QemuCond) {
        let state = get_or_create_mock(cond as usize);
        *state.gen.lock().unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
        state.cv.notify_all();
    }

    pub fn virtmcu_cond_broadcast(cond: *mut QemuCond) {
        let state = get_or_create_mock(cond as usize);
        *state.gen.lock().unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
        state.cv.notify_all();
    }

    pub fn virtmcu_cond_wait(cond: *mut QemuCond, mutex: *mut QemuMutex) {
        virtmcu_cond_timedwait(cond, mutex, u32::MAX);
    }

    pub fn virtmcu_cond_timedwait(cond: *mut QemuCond, mutex: *mut QemuMutex, ms: u32) -> i32 {
        let state = get_or_create_mock(cond as usize);

        // Snapshot the generation counter
        let initial_gen = *state.gen.lock().unwrap_or_else(std::sync::PoisonError::into_inner);

        // Mandate: BQL must NOT be held when calling wait_yielding_bql logic.
        assert!(!virtmcu_bql_locked(), "BQL held during blocking wait!");

        // Atomically release the peripheral mutex before blocking.
        virtmcu_mutex_unlock(mutex);

        // Update waiter count AFTER releasing mutex so tests can synchronize deterministically.
        {
            let mut count =
                state.waiter_count.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            *count += 1;
            state.cv.notify_all();
        }

        let signaled = if ms == u32::MAX {
            let guard = state.gen.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            drop(
                state
                    .cv
                    .wait_while(guard, |g| *g == initial_gen)
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            );
            true
        } else {
            let timeout = core::time::Duration::from_millis(ms as u64);
            let guard = state.gen.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let result = state
                .cv
                .wait_timeout_while(guard, timeout, |g| *g == initial_gen)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            !result.1.timed_out()
        };

        // Re-acquire the peripheral mutex before returning to the caller.
        virtmcu_mutex_lock(mutex);

        // Decrement waiter count.
        {
            let mut count =
                state.waiter_count.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            *count -= 1;
            state.cv.notify_all();
        }

        if signaled {
            1
        } else {
            0
        }
    }
}

/// A safe wrapper for the Big QEMU Lock (BQL).
pub struct Bql;

impl Bql {
    /// Acquires the BQL and returns a guard. The lock is released when the guard is dropped.
    pub fn lock() -> BqlGuard {
        #[cfg(not(any(test, miri)))]
        // SAFETY: virtmcu_bql_lock is a QEMU-provided function to acquire the
        // global lock. It is safe to call from any thread.
        unsafe {
            virtmcu_bql_lock();
        };
        #[cfg(any(test, miri))]
        mock::virtmcu_bql_lock();
        BqlGuard
    }

    /// Acquires the BQL but does NOT return a guard. The lock will remain held.
    /// This is used when transferring lock ownership to a C caller.
    pub fn lock_forget() {
        #[cfg(not(any(test, miri)))]
        // SAFETY: virtmcu_bql_lock is safe to call. Ownership is explicitly
        // managed by the caller.
        unsafe {
            virtmcu_bql_lock();
        };
        #[cfg(any(test, miri))]
        mock::virtmcu_bql_lock();
    }

    /// Explicitly unlocks the BQL. Use this only when you need to block without holding the lock.
    ///
    /// # Safety
    /// The caller must ensure the BQL is currently held.
    pub unsafe fn unlock() {
        #[cfg(not(any(test, miri)))]
        virtmcu_bql_unlock();
        #[cfg(any(test, miri))]
        mock::virtmcu_bql_unlock();
    }

    /// Temporarily unlocks the BQL and returns a guard that will relock it when dropped.
    /// Returns None if the BQL was not held.
    pub fn temporary_unlock() -> Option<BqlUnlockGuard> {
        #[cfg(not(any(test, miri)))]
        // SAFETY: virtmcu_bql_locked is safe to call from any thread to check
        // lock status.
        let was_locked = unsafe { virtmcu_bql_locked() };
        #[cfg(any(test, miri))]
        let was_locked = mock::virtmcu_bql_locked();

        if was_locked {
            #[cfg(not(any(test, miri)))]
            // SAFETY: virtmcu_bql_force_unlock is safe when the lock is held.
            unsafe {
                virtmcu_bql_force_unlock();
            }
            #[cfg(any(test, miri))]
            mock::virtmcu_bql_force_unlock();
            Some(BqlUnlockGuard)
        } else {
            None
        }
    }

    /// Returns true if the BQL is currently held by the calling thread.
    pub fn is_held() -> bool {
        #[cfg(not(any(test, miri)))]
        // SAFETY: Safe FFI call.
        unsafe {
            virtmcu_bql_locked()
        }
        #[cfg(any(test, miri))]
        mock::virtmcu_bql_locked()
    }
}

/// A struct
pub struct BqlGuard;

impl Drop for BqlGuard {
    fn drop(&mut self) {
        #[cfg(not(any(test, miri)))]
        // SAFETY: Releasing the lock is safe if held.
        unsafe {
            virtmcu_bql_unlock();
        };
        #[cfg(any(test, miri))]
        mock::virtmcu_bql_unlock();
    }
}

/// A struct
pub struct BqlUnlockGuard;

impl Drop for BqlUnlockGuard {
    fn drop(&mut self) {
        #[cfg(not(any(test, miri)))]
        // SAFETY: Re-acquiring the lock is safe.
        unsafe {
            virtmcu_bql_force_lock();
        };
        #[cfg(any(test, miri))]
        mock::virtmcu_bql_force_lock();
    }
}

/// A struct
pub struct QemuMutexGuard<'a> {
    pub(crate) mutex: *mut QemuMutex,
    _marker: core::marker::PhantomData<&'a mut QemuMutex>,
}

impl QemuMutexGuard<'_> {
    /// Creates a guard from a raw pointer that is already locked.
    ///
    /// # Safety
    /// The caller must ensure the mutex is already locked and that the lifetime 'a is appropriate.
    pub unsafe fn from_raw(mutex: *mut QemuMutex) -> Self {
        Self { mutex, _marker: core::marker::PhantomData }
    }

    /// Forgets the guard without unlocking the mutex.
    pub fn forget(self) {
        core::mem::forget(self);
    }
}

impl QemuMutex {
    /// A method
    pub fn lock(&mut self) -> QemuMutexGuard<'_> {
        #[cfg(not(any(test, miri)))]
        // SAFETY: Mutex pointer is valid.
        unsafe {
            virtmcu_mutex_lock(core::ptr::from_ref(self).cast_mut());
        };
        #[cfg(any(test, miri))]
        mock::virtmcu_mutex_lock(core::ptr::from_ref(self).cast_mut());
        QemuMutexGuard {
            mutex: core::ptr::from_ref(self).cast_mut(),
            _marker: core::marker::PhantomData,
        }
    }
}

impl Drop for QemuMutexGuard<'_> {
    fn drop(&mut self) {
        #[cfg(not(any(test, miri)))]
        // SAFETY: Mutex pointer is valid and locked by current thread.
        unsafe {
            virtmcu_mutex_unlock(self.mutex);
        };
        #[cfg(any(test, miri))]
        mock::virtmcu_mutex_unlock(self.mutex);
    }
}

impl QemuCond {
    /// A method
    pub fn wait(&self, mutex: &mut QemuMutex) {
        #[cfg(not(any(test, miri)))]
        // SAFETY: Both pointers are valid.
        unsafe {
            virtmcu_cond_wait(core::ptr::from_ref(self).cast_mut(), core::ptr::from_mut(mutex));
        };
        #[cfg(any(test, miri))]
        mock::virtmcu_cond_wait(core::ptr::from_ref(self).cast_mut(), core::ptr::from_mut(mutex));
    }

    /// A method
    pub fn wait_timeout(&self, mutex: &mut QemuMutex, ms: u32) -> bool {
        #[cfg(not(any(test, miri)))]
        // SAFETY: Both pointers are valid.
        unsafe {
            virtmcu_cond_timedwait(
                core::ptr::from_ref(self).cast_mut(),
                core::ptr::from_mut(mutex),
                ms,
            ) != 0
        }
        #[cfg(any(test, miri))]
        {
            mock::virtmcu_cond_timedwait(
                core::ptr::from_ref(self).cast_mut(),
                core::ptr::from_mut(mutex),
                ms,
            ) != 0
        }
    }

    /// A method
    pub fn signal(&self) {
        #[cfg(not(any(test, miri)))]
        // SAFETY: Condition variable pointer is valid.
        unsafe {
            virtmcu_cond_signal(core::ptr::from_ref(self).cast_mut());
        };
        #[cfg(any(test, miri))]
        mock::virtmcu_cond_signal(core::ptr::from_ref(self).cast_mut());
    }

    /// A method
    pub fn broadcast(&self) {
        #[cfg(not(any(test, miri)))]
        // SAFETY: Condition variable pointer is valid.
        unsafe {
            virtmcu_cond_broadcast(core::ptr::from_ref(self).cast_mut());
        };
        #[cfg(any(test, miri))]
        mock::virtmcu_cond_broadcast(core::ptr::from_ref(self).cast_mut());
    }

    /// Atomically releases the BQL, waits on this condition variable using the provided
    /// peripheral mutex guard, and re-acquires the BQL before returning.
    ///
    /// This is the ONLY approved pattern for blocking a vCPU thread while yielding the BQL.
    ///
    /// Returns true on signal/broadcast, false on timeout.
    pub fn wait_yielding_bql(&self, guard: &mut QemuMutexGuard<'_>, timeout_ms: u32) -> bool {
        // 1. Temporarily yield BQL if held.
        let bql_unlock = Bql::temporary_unlock();

        // 2. Wait on the condition variable.
        // SAFETY: We use the raw mutex from the guard.
        let signaled = {
            #[cfg(not(any(test, miri)))]
            // SAFETY: Both pointers are valid.
            unsafe {
                virtmcu_cond_timedwait(
                    core::ptr::from_ref(self).cast_mut(),
                    guard.mutex,
                    timeout_ms,
                ) != 0
            }
            #[cfg(any(test, miri))]
            {
                mock::virtmcu_cond_timedwait(
                    core::ptr::from_ref(self).cast_mut(),
                    guard.mutex,
                    timeout_ms,
                ) != 0
            }
        };

        // 3. To avoid lock order inversion (BQL -> mutex vs mutex -> BQL),
        // we must release the peripheral mutex before re-acquiring the BQL.
        #[cfg(not(any(test, miri)))]
        // SAFETY: Mutex pointer is valid and locked.
        unsafe {
            virtmcu_mutex_unlock(guard.mutex);
        }
        #[cfg(any(test, miri))]
        mock::virtmcu_mutex_unlock(guard.mutex);

        // 4. Re-acquire BQL.
        drop(bql_unlock);

        // 5. Re-acquire peripheral mutex to restore caller's invariants.
        #[cfg(not(any(test, miri)))]
        // SAFETY: Mutex pointer is valid.
        unsafe {
            virtmcu_mutex_lock(guard.mutex);
        }
        #[cfg(any(test, miri))]
        mock::virtmcu_mutex_lock(guard.mutex);

        signaled
    }
}

/// Peripheral state whose invariant is "protected by BQL".
///
/// Replaces `std::sync::Mutex<T>` in QEMU peripheral state structs where every
/// caller already holds the Big QEMU Lock. A real `Mutex` would be correct but
/// misleading — it is always uncontended because BQL serializes every access.
/// `BqlGuarded<T>` makes the protection contract explicit and checks it (debug
/// builds) at every access site.
///
/// # Valid callers
/// Any context that holds BQL is safe:
/// - MMIO read/write handlers (QEMU holds BQL for the TCG vCPU thread)
/// - QEMU timer callbacks (`QomTimer` fires with BQL held)
/// - `SafeSubscriber` callbacks (BQL acquired before the callback is invoked)
///
/// # Do not use for
/// State that must be accessed from Zenoh background threads before BQL can be
/// acquired (e.g., the crossbeam channel sender half). Those need lock-free
/// structures (`AtomicBool`, `crossbeam_channel::Sender`) instead.
pub struct BqlGuarded<T> {
    inner: core::cell::UnsafeCell<T>,
    borrow_count: core::sync::atomic::AtomicIsize,
}

// Safety: The value is only ever read or written while BQL is held, and we use
// dynamic borrow checking to prevent re-entrancy if the BQL is yielded.
// BqlGuarded is Send if the inner type is Send.
// SAFETY: test only
unsafe impl<T: Send> Send for BqlGuarded<T> {}
// Safety: BqlGuarded is Sync if the inner type is Send and Sync, because
// BQL serialization ensures that no two threads can access it simultaneously.
// SAFETY: test only
unsafe impl<T: Send + Sync> Sync for BqlGuarded<T> {}

/// A read guard for `BqlGuarded`.
pub struct BqlReadGuard<'a, T> {
    guarded: &'a BqlGuarded<T>,
    _not_send_sync: core::marker::PhantomData<*mut ()>,
}

impl<T> core::ops::Deref for BqlReadGuard<'_, T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &T {
        debug_assert!(Bql::is_held(), "BQL yielded while holding BqlGuard!");
        // Safety: dynamically checked via borrow_count
        unsafe { &*self.guarded.inner.get() }
    }
}

impl<T> Drop for BqlReadGuard<'_, T> {
    #[inline]
    fn drop(&mut self) {
        self.guarded.borrow_count.fetch_sub(1, core::sync::atomic::Ordering::Release);
    }
}

/// A write guard for `BqlGuarded`.
pub struct BqlWriteGuard<'a, T> {
    guarded: &'a BqlGuarded<T>,
    _not_send_sync: core::marker::PhantomData<*mut ()>,
}

impl<T> core::ops::Deref for BqlWriteGuard<'_, T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &T {
        debug_assert!(Bql::is_held(), "BQL yielded while holding BqlGuard!");
        // Safety: dynamically checked via borrow_count
        unsafe { &*self.guarded.inner.get() }
    }
}

impl<T> core::ops::DerefMut for BqlWriteGuard<'_, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        debug_assert!(Bql::is_held(), "BQL yielded while holding BqlGuard!");
        // Safety: dynamically checked via borrow_count
        unsafe { &mut *self.guarded.inner.get() }
    }
}

impl<T> Drop for BqlWriteGuard<'_, T> {
    #[inline]
    fn drop(&mut self) {
        self.guarded.borrow_count.store(0, core::sync::atomic::Ordering::Release);
    }
}

impl<T> BqlGuarded<T> {
    /// Creates a new `BqlGuarded` value. Does not require BQL.
    pub const fn new(val: T) -> Self {
        Self {
            inner: core::cell::UnsafeCell::new(val),
            borrow_count: core::sync::atomic::AtomicIsize::new(0),
        }
    }

    /// Returns a shared reference guard.
    ///
    /// Debug-asserts that BQL is held. Panics if already mutably borrowed.
    #[inline]
    #[track_caller]
    pub fn get(&self) -> BqlReadGuard<'_, T> {
        debug_assert!(Bql::is_held(), "BqlGuarded::get() called without BQL");
        loop {
            let b = self.borrow_count.load(core::sync::atomic::Ordering::Acquire);
            assert!(b >= 0, "BqlGuarded: already mutably borrowed!");
            if self
                .borrow_count
                .compare_exchange_weak(
                    b,
                    b + 1,
                    core::sync::atomic::Ordering::AcqRel,
                    core::sync::atomic::Ordering::Relaxed,
                )
                .is_ok()
            {
                break;
            }
        }
        BqlReadGuard { guarded: self, _not_send_sync: core::marker::PhantomData }
    }

    /// Returns a mutable reference guard.
    ///
    /// Debug-asserts that BQL is held. Panics if already borrowed.
    #[inline]
    #[track_caller]
    pub fn get_mut(&self) -> BqlWriteGuard<'_, T> {
        debug_assert!(Bql::is_held(), "BqlGuarded::get_mut() called without BQL");
        assert!(
            self.borrow_count
                .compare_exchange(
                    0,
                    -1,
                    core::sync::atomic::Ordering::AcqRel,
                    core::sync::atomic::Ordering::Relaxed,
                )
                .is_ok(),
            "BqlGuarded: already borrowed!"
        );
        BqlWriteGuard { guarded: self, _not_send_sync: core::marker::PhantomData }
    }
}

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use virtmcu_api::{DataCallback, DataTransport};

/// A thread-safe, RAII-enabled subscription for VirtMCU QOM devices.
///
/// It ensures that:
/// 1. The callback always acquires the Big QEMU Lock (BQL).
/// 2. The callback is only executed if the device state is still valid.
/// 3. The callback is only executed if the device generation matches.
/// 4. The subscription is properly cleaned up during drop, preventing Use-After-Free.
pub struct SafeSubscription {
    is_valid: Arc<AtomicBool>,
    active_count: Arc<AtomicUsize>,
    drain_cond: Arc<(std::sync::Mutex<()>, std::sync::Condvar)>,
    _generation: Arc<AtomicU64>,
    _expected_generation: u64,
}

impl SafeSubscription {
    /// Creates a new `SafeSubscription` for the given transport and topic.
    pub fn new<T: DataTransport + ?Sized>(
        transport: &T,
        topic: &str,
        generation: Arc<AtomicU64>,
        callback: DataCallback,
    ) -> Result<Self, String> {
        let expected_generation = generation.load(Ordering::Acquire);
        let generation_clone = Arc::clone(&generation);
        let is_valid = Arc::new(AtomicBool::new(true));
        let valid_clone = Arc::clone(&is_valid);
        let active_count = Arc::new(AtomicUsize::new(0));
        let active_clone = Arc::clone(&active_count);
        let drain_cond = Arc::new((std::sync::Mutex::new(()), std::sync::Condvar::new()));
        let drain_clone = Arc::clone(&drain_cond);

        let wrapper_callback: DataCallback = Box::new(move |payload| {
            // Increment active count before acquiring BQL
            active_clone.fetch_add(1, Ordering::SeqCst);

            {
                // Automatically acquire BQL.
                let _bql = Bql::lock();

                // Re-check validity after acquiring BQL.
                if valid_clone.load(Ordering::Acquire)
                    && generation_clone.load(Ordering::Acquire) == expected_generation
                {
                    callback(payload);
                }
            }

            // Decrement active count when finished.
            active_clone.fetch_sub(1, Ordering::SeqCst);

            // Notify any waiting Drop call that we are done.
            let (lock, cvar) = &*drain_clone;
            if let Ok(_guard) = lock.lock() {
                cvar.notify_all();
            }
        });

        transport.subscribe(topic, wrapper_callback)?;

        Ok(Self {
            is_valid,
            active_count,
            drain_cond,
            _generation: generation,
            _expected_generation: expected_generation,
        })
    }
}

impl Drop for SafeSubscription {
    fn drop(&mut self) {
        // 1. Mark as invalid
        self.is_valid.store(false, Ordering::Release);

        // 2. Temporarily release BQL if held to avoid deadlocks with background threads
        let _unlock = Bql::temporary_unlock();

        // 3. Wait for any remaining active callbacks to finish
        let (lock, cvar) = &*self.drain_cond;
        if let Ok(mut guard) = lock.lock() {
            while self.active_count.load(Ordering::SeqCst) > 0 {
                match cvar.wait(guard) {
                    Ok(new_guard) => guard = new_guard,
                    Err(_) => break,
                }
            }
        }
    }
}

const _: () = assert!(core::mem::size_of::<QemuMutex>() == 64);
const _: () = assert!(core::mem::size_of::<QemuCond>() == 56);

/// A mutual exclusion primitive useful for protecting shared data, based on QEMU's
/// internal `QemuMutex`. It acts exactly like `std::sync::Mutex<T>` but ensures
/// compatibility with QEMU's BQL-yielding wait mechanisms.
pub struct Mutex<T> {
    raw: *mut QemuMutex,
    data: core::cell::UnsafeCell<T>,
}

// SAFETY: Mutex is Send and Sync if the underlying data is Send.
// QemuMutex provides thread safety.
unsafe impl<T: Send> Send for Mutex<T> {}
unsafe impl<T: Send> Sync for Mutex<T> {}

impl<T> Mutex<T> {
    /// Creates a new QEMU-backed mutex in an unlocked state ready for use.
    pub fn new(val: T) -> Self {
        #[cfg(not(any(test, miri)))]
        let raw = unsafe {
            let m = virtmcu_mutex_new();
            qemu_mutex_init(m);
            m
        };
        #[cfg(any(test, miri))]
        let raw = Box::into_raw(Box::new(unsafe { core::mem::zeroed::<QemuMutex>() }));

        Self { raw, data: core::cell::UnsafeCell::new(val) }
    }

    /// Acquires a mutex, blocking the current thread until it is able to do so.
    pub fn lock(&self) -> MutexGuard<'_, T> {
        #[cfg(not(any(test, miri)))]
        unsafe {
            virtmcu_mutex_lock(self.raw);
        };
        #[cfg(any(test, miri))]
        mock::virtmcu_mutex_lock(self.raw);

        MutexGuard { mutex: self }
    }
}

impl<T> Drop for Mutex<T> {
    fn drop(&mut self) {
        #[cfg(not(any(test, miri)))]
        unsafe {
            qemu_mutex_destroy(self.raw);
            virtmcu_mutex_free(self.raw);
        };
        #[cfg(any(test, miri))]
        unsafe {
            let _ = Box::from_raw(self.raw);
        }
    }
}

/// An RAII implementation of a "scoped lock" of a mutex.
pub struct MutexGuard<'a, T> {
    mutex: &'a Mutex<T>,
}

impl<T> core::ops::Deref for MutexGuard<'_, T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &T {
        unsafe { &*self.mutex.data.get() }
    }
}

impl<T> core::ops::DerefMut for MutexGuard<'_, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.mutex.data.get() }
    }
}

impl<T> Drop for MutexGuard<'_, T> {
    fn drop(&mut self) {
        #[cfg(not(any(test, miri)))]
        unsafe {
            virtmcu_mutex_unlock(self.mutex.raw);
        };
        #[cfg(any(test, miri))]
        mock::virtmcu_mutex_unlock(self.mutex.raw);
    }
}

/// A Condition Variable based on QEMU's internal `QemuCond`.
pub struct Condvar {
    raw: *mut QemuCond,
}

// SAFETY: Condvar provides synchronization and does not store thread-local data.
unsafe impl Send for Condvar {}
unsafe impl Sync for Condvar {}

impl Condvar {
    /// Creates a new condition variable which is ready to be waited on and notified.
    pub fn new() -> Self {
        #[cfg(not(any(test, miri)))]
        let raw = unsafe {
            let c = virtmcu_cond_new();
            qemu_cond_init(c);
            c
        };
        #[cfg(any(test, miri))]
        let raw = Box::into_raw(Box::new(unsafe { core::mem::zeroed::<QemuCond>() }));

        Self { raw }
    }

    /// Wakes up one blocked thread on this condvar.
    pub fn notify_one(&self) {
        #[cfg(not(any(test, miri)))]
        unsafe {
            virtmcu_cond_signal(self.raw);
        };
        #[cfg(any(test, miri))]
        mock::virtmcu_cond_signal(self.raw);
    }

    /// Wakes up all blocked threads on this condvar.
    pub fn notify_all(&self) {
        #[cfg(not(any(test, miri)))]
        unsafe {
            virtmcu_cond_broadcast(self.raw);
        };
        #[cfg(any(test, miri))]
        mock::virtmcu_cond_broadcast(self.raw);
    }

    /// Atomically releases the BQL, waits on this condition variable using the provided
    /// peripheral mutex guard, and re-acquires the BQL before returning.
    ///
    /// This is the ONLY approved pattern for blocking a vCPU thread while yielding the BQL
    /// when waiting on a `Mutex<T>`.
    ///
    /// Returns the acquired lock and a boolean (`true` if signaled/spurious, `false` on timeout).
    pub fn wait_yielding_bql<'a, T>(
        &self,
        guard: MutexGuard<'a, T>,
        timeout_ms: u32,
    ) -> (MutexGuard<'a, T>, bool) {
        // 1. Temporarily yield BQL if held.
        let bql_unlock = Bql::temporary_unlock();

        // 2. Wait on the condition variable.
        let signaled = {
            #[cfg(not(any(test, miri)))]
            unsafe {
                virtmcu_cond_timedwait(self.raw, guard.mutex.raw, timeout_ms) != 0
            }
            #[cfg(any(test, miri))]
            {
                mock::virtmcu_cond_timedwait(self.raw, guard.mutex.raw, timeout_ms) != 0
            }
        };

        // 3. To avoid lock order inversion (BQL -> mutex vs mutex -> BQL),
        // we must release the peripheral mutex before re-acquiring the BQL.
        #[cfg(not(any(test, miri)))]
        unsafe {
            virtmcu_mutex_unlock(guard.mutex.raw);
        };
        #[cfg(any(test, miri))]
        mock::virtmcu_mutex_unlock(guard.mutex.raw);

        // 4. Re-acquire BQL.
        drop(bql_unlock);

        // 5. Re-acquire peripheral mutex to restore caller's invariants.
        #[cfg(not(any(test, miri)))]
        unsafe {
            virtmcu_mutex_lock(guard.mutex.raw);
        };
        #[cfg(any(test, miri))]
        mock::virtmcu_mutex_lock(guard.mutex.raw);

        (guard, signaled)
    }

    /// Standard wait without yielding BQL (only safe if BQL is NOT held!).
    pub fn wait<'a, T>(&self, guard: MutexGuard<'a, T>) -> MutexGuard<'a, T> {
        #[cfg(not(any(test, miri)))]
        unsafe {
            virtmcu_cond_wait(self.raw, guard.mutex.raw);
        };
        #[cfg(any(test, miri))]
        mock::virtmcu_cond_wait(self.raw, guard.mutex.raw);
        guard
    }

    /// Standard wait with timeout, without yielding BQL (only safe if BQL is NOT held!).
    pub fn wait_timeout<'a, T>(
        &self,
        guard: MutexGuard<'a, T>,
        ms: u32,
    ) -> (MutexGuard<'a, T>, bool) {
        let res = {
            #[cfg(not(any(test, miri)))]
            unsafe {
                virtmcu_cond_timedwait(self.raw, guard.mutex.raw, ms) != 0
            }
            #[cfg(any(test, miri))]
            {
                mock::virtmcu_cond_timedwait(self.raw, guard.mutex.raw, ms) != 0
            }
        };
        (guard, res)
    }
}

impl Drop for Condvar {
    fn drop(&mut self) {
        #[cfg(not(any(test, miri)))]
        unsafe {
            qemu_cond_destroy(self.raw);
            virtmcu_cond_free(self.raw);
        };
        #[cfg(any(test, miri))]
        unsafe {
            let _ = Box::from_raw(self.raw);
        }
    }
}

impl Default for Condvar {
    fn default() -> Self {
        Self::new()
    }
}

/// An RAII-based drain for tracking active vCPU threads within a peripheral.
///
/// Ensures that during device teardown, QEMU will wait for all blocked MMIO
/// requests to complete, safely yielding the Big QEMU Lock (BQL) during the wait
/// to prevent permanent deadlocks.
pub struct VcpuDrain {
    count: Mutex<usize>,
    cond: Condvar,
}

impl VcpuDrain {
    /// Creates a new, empty vCPU drain tracker.
    pub fn new() -> Self {
        Self { count: Mutex::new(0), cond: Condvar::new() }
    }

    /// Registers an active vCPU. Returns a guard that deregisters it when dropped.
    pub fn acquire(&self) -> VcpuDrainGuard<'_> {
        let mut count = self.count.lock();
        *count = count.saturating_add(1);
        VcpuDrainGuard { drain: self }
    }

    /// Blocks the current thread (yielding the BQL) until the active count reaches 0
    /// or the timeout expires.
    pub fn wait_for_drain(&self, timeout_ms: u32) {
        let mut count = self.count.lock();
        if *count == 0 {
            return;
        }

        #[cfg(not(any(test, miri)))]
        let start_ns =
            unsafe { crate::timer::qemu_clock_get_ns(crate::timer::QEMU_CLOCK_REALTIME) };
        #[cfg(any(test, miri))]
        let start_ns =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
                as i64;

        let limit_ns = (timeout_ms as i64).saturating_mul(1_000_000);

        while *count > 0 {
            #[cfg(not(any(test, miri)))]
            let now_ns =
                unsafe { crate::timer::qemu_clock_get_ns(crate::timer::QEMU_CLOCK_REALTIME) };
            #[cfg(any(test, miri))]
            let now_ns = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos() as i64;

            let elapsed_ns = now_ns.saturating_sub(start_ns);
            if elapsed_ns >= limit_ns {
                crate::sim_err!(
                    "VcpuDrain timed out after {} ms with {} vCPUs still active",
                    timeout_ms,
                    *count
                );
                break;
            }

            let remaining_ms = ((limit_ns - elapsed_ns) / 1_000_000) as u32;
            let (new_count, _) = self.cond.wait_yielding_bql(count, remaining_ms);
            count = new_count;
        }
    }
}

impl Default for VcpuDrain {
    fn default() -> Self {
        Self::new()
    }
}

/// A guard that decrements the `VcpuDrain` count when dropped.
pub struct VcpuDrainGuard<'a> {
    drain: &'a VcpuDrain,
}

impl Drop for VcpuDrainGuard<'_> {
    fn drop(&mut self) {
        let mut count = self.drain.count.lock();
        *count = count.saturating_sub(1);
        if *count == 0 {
            self.drain.cond.notify_all();
        }
    }
}

#[cfg(any(test, miri))]
mod tests {
    use std::thread;

    use super::*;

    struct SyncPtr<T>(*mut T);
    // SAFETY: test only
    unsafe impl<T> Send for SyncPtr<T> {}
    // SAFETY: test only
    unsafe impl<T> Sync for SyncPtr<T> {}
    impl<T> SyncPtr<T> {
        fn get(self) -> *mut T {
            self.0
        }
    }

    #[test]
    fn test_bql_lock_unlock() {
        assert!(Bql::temporary_unlock().is_none());
        {
            let _guard = Bql::lock();
            assert!(Bql::temporary_unlock().is_some());
        }
        assert!(Bql::temporary_unlock().is_none());
    }

    #[test]
    fn test_bql_nested_unlock() {
        let _guard = Bql::lock();
        {
            let unlock = Bql::temporary_unlock();
            assert!(unlock.is_some());
            assert!(Bql::temporary_unlock().is_none());
        }
        assert!(Bql::temporary_unlock().is_some());
    }

    #[test]
    fn test_qemu_mutex_lock() {
        // SAFETY: test only
        let mut mutex: QemuMutex = unsafe { core::mem::zeroed() };
        {
            let _guard = mutex.lock();
        }
    }

    /// Safety boundary for test timeouts to prevent CI hangs.
    const TEST_SAFETY_TIMEOUT_MS: u32 = 10000;

    #[test]
    fn test_wait_yielding_bql_signal() {
        // SAFETY: test only
        let mut mutex: QemuMutex = unsafe { core::mem::zeroed() };
        // SAFETY: test only
        let cond: QemuCond = unsafe { core::mem::zeroed() };

        let cond_ptr = SyncPtr(core::ptr::from_ref(&cond).cast_mut());
        let mutex_ptr = SyncPtr(core::ptr::from_ref(&mutex).cast_mut());

        let _bql = Bql::lock();

        thread::scope(|s| {
            s.spawn(move || {
                // Use .get() to capture the whole SyncPtr and get the raw pointer.
                // This satisfies both the Send bound (capturing the wrapper) and
                // Miri provenance (using the pointer directly).
                let cp = cond_ptr.get();
                let mp = mutex_ptr.get();

                let state = mock::get_or_create_mock(cp as usize);
                let mutex_state = mock::get_or_create_mock(mp as usize);

                // Deterministically wait for the main thread to be blocked.
                {
                    let mut count = state
                        .waiter_count
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    while *count == 0 {
                        count =
                            state.cv.wait(count).unwrap_or_else(std::sync::PoisonError::into_inner);
                    }
                }

                // Assertion: The peripheral mutex must be UNLOCKED while the main thread waits.
                {
                    let locked =
                        mutex_state.mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    assert!(!*locked);
                }

                // Assertion: The BQL must be UNLOCKED (checked via mock state).
                // Note: Since mock BQL is thread-local in this simplified mock, we trust
                // the internal assertion in virtmcu_cond_timedwait.

                // Signal the condition variable.
                // SAFETY: test only
                unsafe { (*cp).signal() };
            });

            let mut guard = mutex.lock();

            // wait_yielding_bql will release BQL, call cond_wait, and re-acquire BQL.
            let res = cond.wait_yielding_bql(&mut guard, TEST_SAFETY_TIMEOUT_MS);
            assert!(res, "Wait should have been signaled deterministically");

            // Verify the peripheral mutex is still held by us.
            drop(guard);
        });

        // Final Assertion: BQL must be held after the wait.
        assert!(Bql::is_held(), "BQL lost after wait_yielding_bql!");
    }

    // ── BqlGuarded tests ────────────────────────────────────────────────

    #[test]
    fn test_bql_guarded_get_returns_value() {
        let guarded = BqlGuarded::new(42u32);
        let _bql = Bql::lock();
        assert_eq!(*guarded.get(), 42);
    }

    #[test]
    fn test_bql_guarded_get_mut_mutates_value() {
        let guarded = BqlGuarded::new(0u64);
        let _bql = Bql::lock();
        *guarded.get_mut() = 99;
        assert_eq!(*guarded.get(), 99);
    }

    #[test]
    fn test_bql_guarded_is_send_and_sync() {
        // Compile-time check: BqlGuarded<T: Send> must be Send + Sync.
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<BqlGuarded<u64>>();
        assert_send_sync::<BqlGuarded<alloc::collections::BinaryHeap<u64>>>();
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "BqlGuarded::get() called without BQL")]
    fn test_bql_guarded_panics_on_get_without_bql() {
        let guarded = BqlGuarded::new(0u32);
        // BQL is NOT held — debug_assert must fire.
        let _ = guarded.get();
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "BqlGuarded::get_mut() called without BQL")]
    fn test_bql_guarded_panics_on_get_mut_without_bql() {
        let guarded = BqlGuarded::new(0u32);
        // BQL is NOT held — debug_assert must fire.
        let _ = guarded.get_mut();
    }

    #[test]
    #[should_panic(expected = "already mutably borrowed")]
    fn test_bql_guarded_read_after_write() {
        let guarded = BqlGuarded::new(42u32);
        let _bql = Bql::lock();
        let _write = guarded.get_mut();
        let _read = guarded.get(); // Panics dynamically
    }

    #[test]
    #[should_panic(expected = "already borrowed")]
    fn test_bql_guarded_write_after_read() {
        let guarded = BqlGuarded::new(42u32);
        let _bql = Bql::lock();
        let _read = guarded.get();
        let _write = guarded.get_mut(); // Panics dynamically
    }

    #[test]
    fn test_vcpu_drain() {
        let drain = VcpuDrain::new();
        assert_eq!(*drain.count.lock(), 0);

        let guard1 = drain.acquire();
        assert_eq!(*drain.count.lock(), 1);

        let guard2 = drain.acquire();
        assert_eq!(*drain.count.lock(), 2);

        drop(guard1);
        assert_eq!(*drain.count.lock(), 1);

        drop(guard2);
        assert_eq!(*drain.count.lock(), 0);
    }

    #[test]
    fn test_vcpu_drain_wait_timeout() {
        let drain = VcpuDrain::new();
        let _guard = drain.acquire();

        let _bql = Bql::lock();
        let start = std::time::Instant::now();
        // This should timeout since we hold the guard
        drain.wait_for_drain(10);
        let elapsed = start.elapsed();

        assert!(elapsed.as_millis() >= 10);
    }
}
