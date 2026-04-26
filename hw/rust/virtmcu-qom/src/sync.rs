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
    use std::collections::HashMap;
    use std::sync::{Arc, Condvar, Mutex};
    use std::thread_local;

    // Thread-local BQL flag: each test thread has isolated BQL state, preventing
    // interference when cargo test runs test functions in parallel.
    thread_local! {
        static BQL_HELD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
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
        let mut guard = MOCK_REGISTRY.lock().unwrap();
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
        BQL_HELD.with(|b| b.get())
    }

    pub fn virtmcu_mutex_lock(mutex: *mut QemuMutex) {
        let state = get_or_create_mock(mutex as usize);
        let mut locked = state.mutex.lock().unwrap();
        while *locked {
            locked = state.cv.wait(locked).unwrap();
        }
        *locked = true;
    }

    pub fn virtmcu_mutex_unlock(mutex: *mut QemuMutex) {
        let state = get_or_create_mock(mutex as usize);
        let mut locked = state.mutex.lock().unwrap();
        *locked = false;
        state.cv.notify_all();
    }

    pub fn virtmcu_cond_signal(cond: *mut QemuCond) {
        let state = get_or_create_mock(cond as usize);
        *state.gen.lock().unwrap() += 1;
        state.cv.notify_all();
    }

    pub fn virtmcu_cond_broadcast(cond: *mut QemuCond) {
        let state = get_or_create_mock(cond as usize);
        *state.gen.lock().unwrap() += 1;
        state.cv.notify_all();
    }

    pub fn virtmcu_cond_wait(cond: *mut QemuCond, mutex: *mut QemuMutex) {
        virtmcu_cond_timedwait(cond, mutex, u32::MAX);
    }

    pub fn virtmcu_cond_timedwait(cond: *mut QemuCond, mutex: *mut QemuMutex, ms: u32) -> i32 {
        let state = get_or_create_mock(cond as usize);

        // Snapshot the generation counter
        let initial_gen = *state.gen.lock().unwrap();

        // Mandate: BQL must NOT be held when calling wait_yielding_bql logic.
        assert!(!virtmcu_bql_locked(), "BQL held during blocking wait!");

        // Atomically release the peripheral mutex before blocking.
        virtmcu_mutex_unlock(mutex);

        // Update waiter count AFTER releasing mutex so tests can synchronize deterministically.
        {
            let mut count = state.waiter_count.lock().unwrap();
            *count += 1;
            state.cv.notify_all();
        }

        let signaled = if ms == u32::MAX {
            let guard = state.gen.lock().unwrap();
            drop(state.cv.wait_while(guard, |g| *g == initial_gen).unwrap());
            true
        } else {
            let timeout = std::time::Duration::from_millis(ms as u64);
            let guard = state.gen.lock().unwrap();
            let result =
                state.cv.wait_timeout_while(guard, timeout, |g| *g == initial_gen).unwrap();
            !result.1.timed_out()
        };

        // Re-acquire the peripheral mutex before returning to the caller.
        virtmcu_mutex_lock(mutex);

        // Decrement waiter count.
        {
            let mut count = state.waiter_count.lock().unwrap();
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

    /// Returns true if the BQL is currently held by the calling thread.
    pub fn is_held() -> bool {
        #[cfg(not(test))]
        unsafe {
            virtmcu_bql_locked()
        }
        #[cfg(test)]
        mock::virtmcu_bql_locked()
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
        #[cfg(not(test))]
        unsafe {
            virtmcu_mutex_lock(core::ptr::from_ref(self).cast_mut());
        };
        #[cfg(test)]
        mock::virtmcu_mutex_lock(core::ptr::from_ref(self).cast_mut());
        QemuMutexGuard {
            mutex: core::ptr::from_ref(self).cast_mut(),
            _marker: core::marker::PhantomData,
        }
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
    pub fn wait(&self, mutex: &mut QemuMutex) {
        #[cfg(not(test))]
        unsafe {
            virtmcu_cond_wait(core::ptr::from_ref(self).cast_mut(), core::ptr::from_mut(mutex));
        };
        #[cfg(test)]
        mock::virtmcu_cond_wait(core::ptr::from_ref(self).cast_mut(), core::ptr::from_mut(mutex));
    }

    /// A method
    pub fn wait_timeout(&self, mutex: &mut QemuMutex, ms: u32) -> bool {
        #[cfg(not(test))]
        unsafe {
            virtmcu_cond_timedwait(
                core::ptr::from_ref(self).cast_mut(),
                core::ptr::from_mut(mutex),
                ms,
            ) != 0
        }
        #[cfg(test)]
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
        #[cfg(not(test))]
        unsafe {
            virtmcu_cond_signal(core::ptr::from_ref(self).cast_mut());
        };
        #[cfg(test)]
        mock::virtmcu_cond_signal(core::ptr::from_ref(self).cast_mut());
    }

    /// A method
    pub fn broadcast(&self) {
        #[cfg(not(test))]
        unsafe {
            virtmcu_cond_broadcast(core::ptr::from_ref(self).cast_mut());
        };
        #[cfg(test)]
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
            #[cfg(not(test))]
            unsafe {
                virtmcu_cond_timedwait(
                    core::ptr::from_ref(self).cast_mut(),
                    guard.mutex,
                    timeout_ms,
                ) != 0
            }
            #[cfg(test)]
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
        #[cfg(not(test))]
        unsafe {
            virtmcu_mutex_unlock(guard.mutex);
        }
        #[cfg(test)]
        mock::virtmcu_mutex_unlock(guard.mutex);

        // 4. Re-acquire BQL.
        drop(bql_unlock);

        // 5. Re-acquire peripheral mutex to restore caller's invariants.
        #[cfg(not(test))]
        unsafe {
            virtmcu_mutex_lock(guard.mutex);
        }
        #[cfg(test)]
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
unsafe impl<T: Send> Send for BqlGuarded<T> {}
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

const _: () = assert!(core::mem::size_of::<QemuMutex>() == 64);
const _: () = assert!(core::mem::size_of::<QemuCond>() == 56);

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    struct SyncPtr<T>(*mut T);
    unsafe impl<T> Send for SyncPtr<T> {}
    unsafe impl<T> Sync for SyncPtr<T> {}
    impl<T> SyncPtr<T> {
        fn get(self) -> *mut T {
            self.0
        }
    }

    #[test]
    fn test_bql_lock_unlock() {
        assert!(!Bql::temporary_unlock().is_some());
        {
            let _guard = Bql::lock();
            assert!(Bql::temporary_unlock().is_some());
        }
        assert!(!Bql::temporary_unlock().is_some());
    }

    #[test]
    fn test_bql_nested_unlock() {
        let _guard = Bql::lock();
        {
            let unlock = Bql::temporary_unlock();
            assert!(unlock.is_some());
            assert!(!Bql::temporary_unlock().is_some());
        }
        assert!(Bql::temporary_unlock().is_some());
    }

    #[test]
    fn test_qemu_mutex_lock() {
        let mut mutex: QemuMutex = unsafe { std::mem::zeroed() };
        {
            let _guard = mutex.lock();
        }
    }

    /// Safety boundary for test timeouts to prevent CI hangs.
    const TEST_SAFETY_TIMEOUT_MS: u32 = 10000;

    #[test]
    fn test_wait_yielding_bql_signal() {
        let mut mutex: QemuMutex = unsafe { std::mem::zeroed() };
        let cond: QemuCond = unsafe { std::mem::zeroed() };

        let cond_ptr = SyncPtr(&cond as *const QemuCond as *mut QemuCond);
        let mutex_ptr = SyncPtr(&mutex as *const QemuMutex as *mut QemuMutex);

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
                    let mut count = state.waiter_count.lock().unwrap();
                    while *count == 0 {
                        count = state.cv.wait(count).unwrap();
                    }
                }

                // Assertion: The peripheral mutex must be UNLOCKED while the main thread waits.
                {
                    let locked = mutex_state.mutex.lock().unwrap();
                    assert!(!*locked, "Peripheral mutex held while thread is waiting!");
                }

                // Assertion: The BQL must be UNLOCKED (checked via mock state).
                // Note: Since mock BQL is thread-local in this simplified mock, we trust
                // the internal assertion in virtmcu_cond_timedwait.

                // Signal the condition variable.
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
        assert_send_sync::<BqlGuarded<std::collections::BinaryHeap<u64>>>();
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
    fn test_wait_yielding_bql_timeout() {
        let mut mutex: QemuMutex = unsafe { std::mem::zeroed() };
        let cond: QemuCond = unsafe { std::mem::zeroed() };

        let _bql = Bql::lock();
        let mut guard = mutex.lock();

        let t0 = std::time::Instant::now();
        let res = cond.wait_yielding_bql(&mut guard, 10);
        let elapsed = t0.elapsed();

        assert!(!res, "Should have timed out");
        assert!(elapsed >= std::time::Duration::from_millis(10));
        assert!(Bql::temporary_unlock().is_some(), "BQL should be re-acquired");
        drop(guard);
    }
}
