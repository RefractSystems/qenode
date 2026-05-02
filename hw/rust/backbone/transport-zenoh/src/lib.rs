#![allow(missing_docs)]
extern crate alloc;

use alloc::format;
use alloc::string::ToString;
use alloc::sync::Arc;
use core::ffi::{c_char, CStr};
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::OnceLock;
use virtmcu_qom::sync::Bql;
use zenoh::pubsub::Subscriber;
use zenoh::{Config, Session, Wait};

use virtmcu_api::DataTransport;

pub mod publisher;
pub use publisher::{SafePublisher, SafeSessionPublisher};

static SHARED_SESSION: OnceLock<Arc<Session>> = OnceLock::new();

/// A Zenoh-backed implementation of the `DataTransport` trait.
pub struct ZenohDataTransport {
    session: Arc<Session>,
    subscriptions: std::sync::Mutex<Vec<Subscriber<()>>>,
}

impl ZenohDataTransport {
    /// Creates a new `ZenohDataTransport` using the provided Zenoh session.
    pub fn new(session: Arc<Session>) -> Self {
        Self { session, subscriptions: std::sync::Mutex::new(Vec::new()) }
    }
}

impl DataTransport for ZenohDataTransport {
    fn publish(&self, topic: &str, payload: &[u8]) -> Result<(), String> {
        self.session.put(topic, payload).wait().map_err(|e| e.to_string())
    }

    fn subscribe(&self, topic: &str, callback: virtmcu_api::DataCallback) -> Result<(), String> {
        let sub = self
            .session
            .declare_subscriber(topic)
            .callback(move |sample| {
                callback(sample.payload().to_bytes().as_ref());
            })
            .wait()
            .map_err(|e| e.to_string())?;
        self.subscriptions.lock().unwrap().push(sub);
        Ok(())
    }

    fn query(&self, topic: &str, payload: &[u8]) -> Result<Vec<u8>, String> {
        let replies = self.session.get(topic).payload(payload).wait().map_err(|e| e.to_string())?;
        while let Ok(reply) = replies.recv() {
            if let Ok(sample) = reply.result() {
                return Ok(sample.payload().to_bytes().to_vec());
            }
        }
        Err("No reply received".to_owned())
    }
}

/// Returns a shared Zenoh session, initializing it if necessary.
///
/// This implements the Shared Zenoh Session Pool.
///
/// **NOTE:** This session is intended for DATA PLANE use only (UART, CAN, SPI, etc.).
/// The `clock` peripheral MUST use its own dedicated session via `open_session`
/// to ensure priority isolation and avoid starvation.
///
/// # Safety
///
/// The caller must ensure that `router` is either NULL or a valid, null-terminated
/// C string if this is the first call to this function.
pub unsafe fn get_or_init_session(router: *const c_char) -> Result<Arc<Session>, zenoh::Error> {
    if let Some(session) = SHARED_SESSION.get() {
        return Ok(Arc::clone(session));
    }

    // SAFETY: router validity is guaranteed by the caller.
    let session = Arc::new(unsafe { open_session(router)? });
    match SHARED_SESSION.set(Arc::clone(&session)) {
        Ok(()) => Ok(session),
        Err(existing) => Ok(Arc::clone(&existing)),
    }
}

/// A thread-safe, RAII-enabled Zenoh subscriber for VirtMCU QOM devices.
///
/// It ensures that:
/// 1. The callback always acquires the Big QEMU Lock (BQL).
/// 2. The callback is only executed if the device state is still valid.
/// 3. The callback is only executed if the device generation matches.
/// 4. The subscriber is properly undeclared and synchronization occurs during drop,
///    preventing Use-After-Free during device finalization.
pub struct SafeSubscriber {
    subscriber: Option<Subscriber<()>>,
    is_valid: Arc<AtomicBool>,
    active_count: Arc<AtomicUsize>,
    drain_cond: Arc<(std::sync::Mutex<()>, std::sync::Condvar)>,
    generation: Arc<AtomicU64>,
    expected_generation: u64,
}

impl SafeSubscriber {
    /// Creates a new `SafeSubscriber`.
    ///
    /// This is a legacy wrapper for backward compatibility. Use `new_with_generation`
    /// to provide a shared generation counter for stale message detection.
    ///
    /// # Arguments
    /// * `session` - The Zenoh session.
    /// * `topic` - The topic to subscribe to.
    /// * `callback` - The closure to execute when a sample is received.
    ///   The BQL is already held when this callback runs.
    ///
    /// # Errors
    /// Returns a Zenoh error if the subscriber declaration fails.
    pub fn new<F>(session: &Session, topic: &str, callback: F) -> Result<Self, zenoh::Error>
    where
        F: Fn(zenoh::sample::Sample) + Send + Sync + 'static,
    {
        // Default to a dummy generation counter for legacy callers.
        let generation = Arc::new(AtomicU64::new(0));
        Self::new_with_generation(session, topic, generation, callback)
    }

    /// Creates a new `SafeSubscriber` with a generation counter.
    ///
    /// # Arguments
    /// * `session` - The Zenoh session.
    /// * `topic` - The topic to subscribe to.
    /// * `generation` - The generation counter shared with the device.
    /// * `callback` - The closure to execute when a sample is received.
    ///   The BQL is already held when this callback runs.
    ///
    /// # Errors
    /// Returns a Zenoh error if the subscriber declaration fails.
    pub fn new_with_generation<F>(
        session: &Session,
        topic: &str,
        generation: Arc<AtomicU64>,
        callback: F,
    ) -> Result<Self, zenoh::Error>
    where
        F: Fn(zenoh::sample::Sample) + Send + Sync + 'static,
    {
        let expected_generation = generation.load(Ordering::Acquire);
        let generation_clone = Arc::clone(&generation);
        let is_valid = Arc::new(AtomicBool::new(true));
        let valid_clone = Arc::clone(&is_valid);
        let active_count = Arc::new(AtomicUsize::new(0));
        let active_clone = Arc::clone(&active_count);
        let drain_cond = Arc::new((std::sync::Mutex::new(()), std::sync::Condvar::new()));
        let drain_clone = Arc::clone(&drain_cond);

        let subscriber = session
            .declare_subscriber(topic)
            .callback(move |sample| {
                // Increment active count before acquiring BQL to signal we are starting.
                active_clone.fetch_add(1, Ordering::SeqCst);

                {
                    // Automatically acquire BQL.
                    let _bql = Bql::lock();

                    // Re-check validity after acquiring BQL.
                    if valid_clone.load(Ordering::Acquire) {
                        // Check if the message is from a stale device generation.
                        if generation_clone.load(Ordering::Acquire) == expected_generation {
                            callback(sample);
                        } else {
                            // Message belongs to a previous device lifetime.
                            virtmcu_qom::sim_trace!("SafeSubscriber dropped stale message");
                        }
                    }
                }

                // Decrement active count when finished.
                active_clone.fetch_sub(1, Ordering::SeqCst);

                // Notify any waiting Drop call that we are done.
                let (lock, cvar) = &*drain_clone;
                if let Ok(_guard) = lock.lock() {
                    cvar.notify_all();
                }
            })
            .wait()?;

        Ok(Self {
            subscriber: Some(subscriber),
            is_valid,
            active_count,
            drain_cond,
            generation,
            expected_generation,
        })
    }

    /// Returns the current value of the shared generation counter.
    pub fn current_generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Returns the generation this subscriber was created with.
    pub fn expected_generation(&self) -> u64 {
        self.expected_generation
    }
}

impl Drop for SafeSubscriber {
    fn drop(&mut self) {
        // 1. Mark as invalid so no NEW callbacks proceed to execute the inner closure.
        self.is_valid.store(false, Ordering::Release);

        // 2. Temporarily release the BQL if we hold it. This is CRITICAL because:
        //    a) The Zenoh background thread might be blocked on BQL in its callback wrapper.
        //    b) Zenoh's undeclare().wait() might wait for that callback to finish.
        //    c) Device finalization always happens UNDER the BQL.
        //    Without this, we deadlock.
        let _unlock = Bql::temporary_unlock();

        // 3. Undeclare and wait for network/task ack.
        if let Some(sub) = self.subscriber.take() {
            let _ = sub.undeclare().wait();
        }

        // 4. Wait for any remaining active callbacks to finish their wrapper body.
        //    This ensures that when Drop returns, the captured variables (like raw state pointers)
        //    are no longer being accessed by any Zenoh thread.
        let (lock, cvar) = &*self.drain_cond;
        if let Ok(mut guard) = lock.lock() {
            while self.active_count.load(Ordering::SeqCst) > 0 {
                match cvar.wait(guard) {
                    Ok(new_guard) => guard = new_guard,
                    Err(_) => break, // Mutex poisoned; best-effort exit.
                }
            }
        }
    }
}

/// Opens a Zenoh session with a standardized config for virtmcu.
///
/// If `router` is provided and non-empty, it is used as a connect endpoint.
/// Scouting is disabled if a router is provided.
///
/// # Safety
///
/// The caller must ensure that `router` is either NULL or a valid, null-terminated
/// C string that remains valid for the duration of this call.
pub unsafe fn open_session(router: *const c_char) -> Result<Session, zenoh::Error> {
    let mut config = Config::default();
    let mut has_router = false;

    // Task 4.2: High-performance executor for co-simulation
    let _ = config.insert_json5("task_planning/concurrency", "8");

    if !router.is_null() {
        // SAFETY: The caller must ensure router is valid. We perform a best-effort
        // check for null-termination to avoid runaway reads.
        let mut len = 0;
        while len < 1024 {
            if unsafe { *router.add(len) } == 0 {
                break;
            }
            len += 1;
        }
        if len == 1024 {
            return Err(zenoh::Error::from(
                "router endpoint too long or not null-terminated (max 1024)",
            ));
        }

        // SAFETY: The caller guarantees that router is a valid null-terminated C string.
        let r_str = unsafe { CStr::from_ptr(router) }
            .to_str()
            .map_err(|e| zenoh::Error::from(e.to_string()))?;
        if !r_str.is_empty() {
            let json = format!("[\"{r_str}\"]");
            let _ = config.insert_json5("mode", "\"client\"");
            let _ = config.insert_json5("connect/endpoints", &json);
            let _ = config.insert_json5("scouting/multicast/enabled", "false");
            let _ = config.insert_json5("transport/shared_memory/enabled", "false");
            has_router = true;
        }
    }

    let session = zenoh::open(config)
        .wait()
        .map_err(|e| zenoh::Error::from(format!("Failed to open Zenoh session: {e}")))?;
    virtmcu_qom::vlog!("transport-zenoh: zenoh::open() finished. has_router={}", has_router);

    // If a router was provided, log it.
    if has_router {
        virtmcu_qom::sim_info!("Connected to Zenoh topology.");
    }

    Ok(session)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::{AtomicU64, AtomicUsize};
    use core::time::Duration;

    // Mocks for BQL functions normally provided by QEMU.
    // These are needed because virtmcu-qom/src/ffi.c calls them when UNIT_TEST is not defined.
    static MOCK_BQL: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

    std::thread_local! {
        static BQL_HELD_BY_ME: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
    }

    #[no_mangle]
    extern "C" fn virtmcu_is_bql_locked() -> bool {
        BQL_HELD_BY_ME.with(core::cell::Cell::get)
    }
    #[no_mangle]
    extern "C" fn virtmcu_safe_bql_lock() {
        if virtmcu_is_bql_locked() {
            return; // Mock recursive lock
        }
        while MOCK_BQL
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            std::thread::yield_now();
        }
        BQL_HELD_BY_ME.with(|b| b.set(true));
    }
    #[no_mangle]
    extern "C" fn virtmcu_safe_bql_unlock() {
        if virtmcu_is_bql_locked() {
            BQL_HELD_BY_ME.with(|b| b.set(false));
            MOCK_BQL.store(false, Ordering::Release);
        }
    }
    #[no_mangle]
    extern "C" fn virtmcu_safe_bql_force_lock() {
        virtmcu_safe_bql_lock();
    }
    #[no_mangle]
    extern "C" fn virtmcu_safe_bql_force_unlock() {
        virtmcu_safe_bql_unlock();
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_safe_subscriber_lifecycle() -> Result<(), zenoh::Error> {
        let config = Config::default();
        // Use memory transport for fast unit tests
        let session = zenoh::open(config).wait().map_err(|e| zenoh::Error::from(e.to_string()))?;
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);
        let generation = Arc::new(AtomicU64::new(0));

        let topic = "tests/fixtures/guest_apps/safe/sub";

        {
            let _sub =
                SafeSubscriber::new_with_generation(&session, topic, generation, move |_sample| {
                    counter_clone.fetch_add(1, Ordering::SeqCst);
                })?;

            // Publish a message
            session.put(topic, "hello").wait().map_err(|e| zenoh::Error::from(e.to_string()))?;

            // Wait for callback (it might take a moment as it's async)
            let mut attempts = 0;
            while counter.load(Ordering::SeqCst) == 0 && attempts < 100 {
                let d = Duration::from_millis(10);
                std::thread::sleep(d); // SLEEP_EXCEPTION: test-only; polling for async Zenoh callback (wall-clock boundary test).
                attempts += 1;
            }
            assert!(counter.load(Ordering::SeqCst) > 0);
        }

        // Sub is now dropped. Marking it as invalid and undeclaring should have happened.
        let count_after_drop = counter.load(Ordering::SeqCst);

        // Publish more - should NOT be received
        for _ in 0..10 {
            session.put(topic, "ignored").wait().map_err(|e| zenoh::Error::from(e.to_string()))?;
        }

        let d = Duration::from_millis(100);
        std::thread::sleep(d); // SLEEP_EXCEPTION: test-only; verifying quiescence after subscriber drop (wall-clock boundary test).
        assert_eq!(counter.load(Ordering::SeqCst), count_after_drop);
        Ok(())
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_safe_subscriber_drain_completes_under_load() -> Result<(), zenoh::Error> {
        let config = Config::default();
        let session = zenoh::open(config).wait().map_err(|e| zenoh::Error::from(e.to_string()))?;
        let counter = Arc::new(AtomicUsize::new(0));
        let topic = "tests/fixtures/guest_apps/stress/drain";
        let generation = Arc::new(AtomicU64::new(0));

        // Create a SafeSubscriber whose callback has a slight delay to simulate load
        let counter_clone = Arc::clone(&counter);
        let sub =
            SafeSubscriber::new_with_generation(&session, topic, generation, move |_sample| {
                // Simulating workload that takes some time
                std::thread::sleep(Duration::from_millis(1)); // SLEEP_EXCEPTION: test-only; simulating workload
                counter_clone.fetch_add(1, Ordering::SeqCst);
            })?;

        // Spawn threads to publish many messages
        let mut handles = vec![];
        for _ in 0..8 {
            let session_clone = session.clone();
            let handle = std::thread::spawn(move || {
                for _ in 0..20 {
                    let _ = session_clone.put(topic, "data").wait();
                }
            });
            handles.push(handle);
        }

        // Wait a tiny bit for some callbacks to start
        std::thread::sleep(Duration::from_millis(10)); // SLEEP_EXCEPTION: test-only

        // Drop the subscriber while messages are still being processed
        drop(sub);

        // After drop returns, active_count MUST be 0 and no more increments should happen
        let final_count = counter.load(Ordering::SeqCst);

        // Wait to be sure no late callbacks arrive
        std::thread::sleep(Duration::from_millis(100)); // SLEEP_EXCEPTION: test-only
        assert_eq!(counter.load(Ordering::SeqCst), final_count, "Counter increased after Drop!");

        for h in handles {
            let _ = h.join();
        }
        Ok(())
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_generation_drop_stale_callback() -> Result<(), zenoh::Error> {
        let config = Config::default();
        let session = zenoh::open(config).wait().map_err(|e| zenoh::Error::from(e.to_string()))?;
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);
        let generation = Arc::new(AtomicU64::new(0));
        let topic = "tests/fixtures/guest_apps/gen/stale";

        // Create subscriber with gen 0
        let _sub = SafeSubscriber::new_with_generation(
            &session,
            topic,
            Arc::clone(&generation),
            move |_sample| {
                counter_clone.fetch_add(1, Ordering::SeqCst);
            },
        )?;

        // Increment generation to 1 BEFORE publishing (or before callback fires)
        generation.store(1, Ordering::SeqCst);

        session.put(topic, "stale").wait().map_err(|e| zenoh::Error::from(e.to_string()))?;

        // Wait a bit to ensure it would have fired
        std::thread::sleep(Duration::from_millis(100)); // SLEEP_EXCEPTION: test-only
        assert_eq!(counter.load(Ordering::SeqCst), 0, "Stale callback was invoked!");
        Ok(())
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_generation_accepts_current() -> Result<(), zenoh::Error> {
        let config = Config::default();
        let session = zenoh::open(config).wait().map_err(|e| zenoh::Error::from(e.to_string()))?;
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);
        let generation = Arc::new(AtomicU64::new(2));
        let topic = "tests/fixtures/guest_apps/gen/current";

        // Create subscriber with gen 2
        let _sub = SafeSubscriber::new_with_generation(
            &session,
            topic,
            Arc::clone(&generation),
            move |_sample| {
                counter_clone.fetch_add(1, Ordering::SeqCst);
            },
        )?;

        session.put(topic, "valid").wait().map_err(|e| zenoh::Error::from(e.to_string()))?;

        // Wait for callback
        let mut attempts = 0;
        while counter.load(Ordering::SeqCst) == 0 && attempts < 100 {
            std::thread::sleep(Duration::from_millis(10)); // SLEEP_EXCEPTION: test-only
            attempts += 1;
        }
        assert!(counter.load(Ordering::SeqCst) > 0, "Valid callback was NOT invoked!");
        Ok(())
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_shared_session() -> Result<(), zenoh::Error> {
        // SAFETY: Using null for router is safe.
        let s1 = unsafe { get_or_init_session(core::ptr::null()) }
            .map_err(|e| zenoh::Error::from(e.to_string()))?;
        // SAFETY: Using null for router is safe.
        let s2 = unsafe { get_or_init_session(core::ptr::null()) }
            .map_err(|e| zenoh::Error::from(e.to_string()))?;

        assert!(Arc::ptr_eq(&s1, &s2));
        Ok(())
    }

    #[test]
    fn test_open_session_rejects_non_nul() {
        let buffer = [b'a' as c_char; 1024];
        let res = unsafe { open_session(buffer.as_ptr()) };
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("not null-terminated"));
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_generation_stress() -> Result<(), zenoh::Error> {
        let config = Config::default();
        let session = zenoh::open(config).wait().map_err(|e| zenoh::Error::from(e.to_string()))?;
        let generation = Arc::new(AtomicU64::new(0));
        let topic = "tests/fixtures/guest_apps/gen/stress";

        let _sub = SafeSubscriber::new_with_generation(
            &session,
            topic,
            Arc::clone(&generation),
            move |_sample| {
                // Just do some work
                let _ = 1 + 1;
            },
        )?;

        // Rapidly publish and increment generation to flush out race conditions
        for i in 0..100 {
            session.put(topic, "stress").wait().map_err(|e| zenoh::Error::from(e.to_string()))?;
            if i % 10 == 0 {
                generation.fetch_add(1, Ordering::SeqCst);
            }
        }

        Ok(())
    }
}
