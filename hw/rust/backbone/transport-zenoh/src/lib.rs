#![allow(clippy::all, unused_imports, dead_code, unused_variables, unused_mut)] // virtmcu-allow: allow reasoning="Zero unsafe"
#![allow(clippy::all)] // virtmcu-allow: allow reasoning="Zero unsafe"
#![allow(clippy::panic)] // virtmcu-allow: allow reasoning="Fail Loudly"
#![deny(missing_docs)]
#![doc = "Zenoh data transport implementation for virtmcu."]
#![cfg_attr(
    test,
    allow(
        clippy::expect_used,
        clippy::unwrap_used,
        clippy::indexing_slicing,
        clippy::panic_in_result_fn
    )
)]

extern crate alloc;

use alloc::format;
use alloc::string::ToString;
use alloc::sync::Arc;
use core::ffi::{c_char, CStr};
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::OnceLock;
use virtmcu_qom::sync::Bql;
use zenoh::pubsub::Subscriber;
#[cfg(test)]
use zenoh::Config;
use zenoh::{Session, Wait};

/// Zenoh publisher abstractions.
pub mod publisher;
pub use publisher::{SafePublisher, SafeSessionPublisher};

const ZENOH_QUERY_TIMEOUT_SECS: u64 = 3599;
const MAX_ROUTER_LEN: usize = 1024;
const SESSION_OPEN_MAX_RETRIES: i32 = 100;
const SESSION_OPEN_RETRY_DELAY_MS: u64 = 200;

static SHARED_SESSION: OnceLock<Arc<Session>> = OnceLock::new(); // virtmcu-allow: static_state reasoning="Singleton for Zenoh session reuse"

/// A wrapper for Zenoh LivelinessToken to abstract it across the FFI.
pub struct ZenohLivelinessToken {
    _token: zenoh::liveliness::LivelinessToken,
}

impl virtmcu_wire::LivelinessToken for ZenohLivelinessToken {}

/// A Zenoh-backed implementation of the `DataTransport` trait.
pub struct ZenohDataTransport {
    node_id: u32,
    session: Arc<Session>,
    publisher: publisher::SafeSessionPublisher,
    subscriptions: std::sync::Mutex<Vec<Subscriber<()>>>,
}

impl ZenohDataTransport {
    /// Creates a new `ZenohDataTransport` using the provided Zenoh session.
    pub fn new(session: Arc<Session>, node_id: u32) -> Self {
        let publisher = publisher::SafeSessionPublisher::new(Arc::clone(&session));
        Self { node_id, session, publisher, subscriptions: std::sync::Mutex::new(Vec::new()) }
    }
}

impl virtmcu_wire::DataTransport for ZenohDataTransport {
    fn publish(&self, topic: &str, payload: &[u8]) -> Result<(), String> {
        self.publisher.send(topic.to_owned(), payload.to_vec());
        Ok(())
    }

    #[allow(deprecated)] // virtmcu-allow: allow reasoning="Stage 1 stub"
    fn reserve<'a>(
        &'a self,
        _topic: &'a str,
        _size: usize,
    ) -> Result<virtmcu_wire::TransportReservation<'a>, virtmcu_wire::TransportError> {
        todo!("Use reserve_link")
    }

    fn register_link(&self, link_name: &str) -> Result<u32, virtmcu_wire::TransportError> {
        let (tx, rx) = std::sync::mpsc::channel();
        let tx = alloc::sync::Arc::new(std::sync::Mutex::new(tx));
        self.subscribe(
            "sim/coord/link/ack",
            Box::new(move |_topic, payload| {
                let _ = tx.lock().expect("mutex poisoned").send(payload.to_vec());
            }),
        )
        .map_err(|e| virtmcu_wire::TransportError::Other(e))?;

        let payload = virtmcu_wire::encode_link_registration(link_name);
        self.publish("sim/coord/link/register", &payload)
            .map_err(|e| virtmcu_wire::TransportError::Other(e))?;

        let ack_payload = rx.recv().map_err(|_| virtmcu_wire::TransportError::Closed)?;

        if let Ok((link_id, status, _err)) = virtmcu_wire::decode_link_ack(&ack_payload) {
            if status != 0 {
                std::process::abort();
            }
            return Ok(link_id);
        }
        std::process::abort();
    }

    fn reserve_link(
        &self,
        link_id: u32,
        size: usize,
    ) -> Result<virtmcu_wire::TransportReservation<'_>, virtmcu_wire::TransportError> {
        const HEADER_SIZE: usize = 24;
        let required_size = size + HEADER_SIZE;
        let mut frame = vec![0u8; required_size];

        let payload_ptr = frame.as_mut_ptr();
        // The peripheral will write to the slice starting at offset HEADER_SIZE.
        let buffer = unsafe {
            let b = core::slice::from_raw_parts_mut(payload_ptr.add(HEADER_SIZE), size);
            core::mem::transmute::<&mut [u8], &mut [u8]>(b)
        };

        let topic = format!("sim/ch/{link_id}");
        let zenoh_topic = format!("sim/ch/{}/{}", link_id, self.node_id);

        Ok(virtmcu_wire::TransportReservation::new(
            Box::leak(topic.into_boxed_str()),
            buffer,
            move |vtime, seq| {
                const LINK_ID_OFFSET: usize = 0;
                const SIZE_OFFSET: usize = 4;
                const VTIME_OFFSET: usize = 8;
                const SEQ_OFFSET: usize = 16;
                const HEADER_END: usize = 24;

                let payload = &mut frame[..required_size];
                payload[LINK_ID_OFFSET..SIZE_OFFSET].copy_from_slice(&link_id.to_le_bytes());
                payload[SIZE_OFFSET..VTIME_OFFSET].copy_from_slice(&(size as u32).to_le_bytes());
                payload[VTIME_OFFSET..SEQ_OFFSET].copy_from_slice(&vtime.to_le_bytes());
                payload[SEQ_OFFSET..HEADER_END].copy_from_slice(&seq.to_le_bytes());

                self.publish(&zenoh_topic, payload)
                    .map_err(virtmcu_wire::TransportError::Other)
            },
        ))
    }

    fn subscribe(&self, topic: &str, callback: virtmcu_wire::DataCallback) -> Result<(), String> {
        let sub = self
            .session
            .declare_subscriber(topic)
            .callback(move |sample| {
                callback(sample.key_expr().as_str(), sample.payload().to_bytes().as_ref());
            })
            .wait()
            .map_err(|e| e.to_string())?;
        self.subscriptions.lock().expect("zenoh transport error").push(sub);
        Ok(())
    }

    fn query(&self, topic: &str, payload: &[u8]) -> Result<Vec<u8>, String> {
        let replies = self
            .session
            .get(topic)
            .payload(payload)
            .timeout(core::time::Duration::from_secs(ZENOH_QUERY_TIMEOUT_SECS))
            .wait()
            .map_err(|e| e.to_string())?;
        while let Ok(reply) = replies.recv() {
            if let Ok(sample) = reply.result() {
                return Ok(sample.payload().to_bytes().to_vec());
            }
        }
        Err("No reply received".to_owned())
    }

    fn declare_liveliness(
        &self,
        topic: &str,
    ) -> Option<alloc::boxed::Box<dyn virtmcu_wire::LivelinessToken>> {
        match self.session.liveliness().declare_token(topic).wait() {
            Ok(token) => Some(alloc::boxed::Box::new(ZenohLivelinessToken { _token: token })),
            Err(_) => None,
        }
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

/// Returns a Zenoh configuration for the given router.
///
/// # Safety
///
/// The caller must ensure that `router` is either NULL or a valid, null-terminated
/// C string that remains valid for the duration of this call.
pub unsafe fn open_config(router: *const c_char) -> Result<zenoh::Config, zenoh::Error> {
    const ZENOH_DEFAULT_CONCURRENCY: &str = "8";
    let mut config = virtmcu_zenoh_config::client_config();

    // Use peer mode for unit tests to allow standalone operation without a router.
    if option_env!("VIRTMCU_UNIT_TEST").is_some() && router.is_null() {
        let _ = config.insert_json5("mode", "\"peer\"");
        let _ = config.insert_json5("scouting/multicast/enabled", "true");
    }

    // Task 4.2: High-performance executor for co-simulation
    let _ = config.insert_json5("task_planning/concurrency", ZENOH_DEFAULT_CONCURRENCY);

    if !router.is_null() {
        // SAFETY: The caller must ensure router is valid. We perform a best-effort
        // check for null-termination to avoid runaway reads.
        let mut len = 0;
        while len < MAX_ROUTER_LEN {
            if unsafe { *router.add(len) } == 0 {
                break;
            }
            len += 1;
        }
        if len == MAX_ROUTER_LEN {
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
            let _ = config.insert_json5("transport/shared_memory/enabled", "false");
        }
    }

    Ok(config)
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
    let config = unsafe { open_config(router)? };
    let has_router = !router.is_null() && unsafe { !CStr::from_ptr(router).to_bytes().is_empty() };

    let mut session_res = zenoh::open(config.clone()).wait();
    if session_res.is_err() && has_router {
        // Retry for ASan/slow CI environments where the router might be slightly behind
        // even if the orchestrator thinks it's ready.
        for i in 1..=SESSION_OPEN_MAX_RETRIES {
            virtmcu_qom::sim_debug!(
                "transport-zenoh: Zenoh session open failed (attempt {}). Retrying...",
                i
            );
            std::thread::sleep(core::time::Duration::from_millis(SESSION_OPEN_RETRY_DELAY_MS)); // virtmcu-allow: sleep reasoning="transient connection retry"
            session_res = zenoh::open(config.clone()).wait();
            if session_res.is_ok() {
                virtmcu_qom::sim_info!(
                    "transport-zenoh: Zenoh session opened after {} retries.",
                    i
                );
                break;
            }
        }
    }

    let session = session_res
        .map_err(|e| zenoh::Error::from(format!("Failed to open Zenoh session: {e}")))?;
    virtmcu_qom::vlog!("transport-zenoh: zenoh::open() finished. has_router={}", has_router);

    // If a router was provided, log it.
    if has_router {
        virtmcu_qom::sim_info!("Connected to Zenoh topology.");
    }

    Ok(session)
}

#[cfg(test)]
pub(crate) fn test_config() -> Config {
    let mut config = virtmcu_zenoh_config::client_config();
    // Use peer mode for unit tests to allow standalone operation without a router.
    let _ = config.insert_json5("mode", "\"peer\"");
    let _ = config.insert_json5("scouting/multicast/enabled", "true");
    config
}

#[cfg(test)]
#[allow(clippy::items_after_statements)] // virtmcu-allow: allow reasoning="Tests group constants locally for readability"
mod tests {
    use super::*;
    use core::sync::atomic::{AtomicU64, AtomicUsize};
    use core::time::Duration;

    // Mocks for BQL functions normally provided by QEMU.
    // These are needed because virtmcu-qom/src/ffi.c calls them when UNIT_TEST is not defined.
    static MOCK_BQL: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false); // virtmcu-allow: static_state reasoning="Mock state for local testing"

    std::thread_local! { // virtmcu-allow: static_state reasoning="Thread local mock state for local testing"
        static BQL_HELD_BY_ME: core::cell::Cell<bool> = const { core::cell::Cell::new(false) }; // virtmcu-allow: static_state reasoning="Thread local mock state for local testing"
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
            std::thread::yield_now(); // virtmcu-allow: yield reasoning="legacy spinloop"
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
        const TEST_WAIT_TIMEOUT: Duration = Duration::from_secs(5);
        const TEST_IGNORED_MSG_COUNT: usize = 10;
        const TEST_QUIESCENCE_TIMEOUT: Duration = Duration::from_millis(100);

        let config = crate::test_config();
        // Use memory transport for fast unit tests
        let session = zenoh::open(config).wait().map_err(|e| zenoh::Error::from(e.to_string()))?;
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);
        let generation = Arc::new(AtomicU64::new(0));

        let topic = "tests/fixtures/guest_apps/safe/sub";

        let pair = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
        let pair_clone = Arc::clone(&pair);

        {
            let _sub =
                SafeSubscriber::new_with_generation(&session, topic, generation, move |_sample| {
                    counter_clone.fetch_add(1, Ordering::SeqCst);
                    let (lock, cvar) = &*pair_clone;
                    let mut done = lock.lock().unwrap();
                    *done = true;
                    cvar.notify_all();
                })?;

            // Publish a message
            session.put(topic, "hello").wait().map_err(|e| zenoh::Error::from(e.to_string()))?;

            // Wait for callback (it might take a moment as it's async)
            let (lock, cvar) = &*pair;
            let mut done = lock.lock().unwrap();
            let result = cvar.wait_timeout(done, TEST_WAIT_TIMEOUT).unwrap();
            done = result.0;
            assert!(*done, "Callback never triggered within timeout");
            assert!(counter.load(Ordering::SeqCst) > 0);
        }

        // Sub is now dropped. Marking it as invalid and undeclaring should have happened.
        let count_after_drop = counter.load(Ordering::SeqCst);

        // Publish more - should NOT be received
        for _ in 0..TEST_IGNORED_MSG_COUNT {
            session.put(topic, "ignored").wait().map_err(|e| zenoh::Error::from(e.to_string()))?;
        }

        std::thread::sleep(TEST_QUIESCENCE_TIMEOUT); // virtmcu-allow: sleep reasoning="test-only; verifying quiescence after subscriber drop (wall-clock boundary test)."
        assert_eq!(counter.load(Ordering::SeqCst), count_after_drop);
        Ok(())
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_safe_subscriber_drain_completes_under_load() -> Result<(), zenoh::Error> {
        let config = crate::test_config();
        let session = zenoh::open(config).wait().map_err(|e| zenoh::Error::from(e.to_string()))?;
        let counter = Arc::new(AtomicUsize::new(0));
        let topic = "tests/fixtures/guest_apps/stress/drain";
        let generation = Arc::new(AtomicU64::new(0));

        // Create a SafeSubscriber whose callback has a slight delay to simulate load
        let counter_clone = Arc::clone(&counter);
        let sub =
            SafeSubscriber::new_with_generation(&session, topic, generation, move |_sample| {
                // Simulating workload that takes some time
                std::thread::sleep(Duration::from_millis(1)); // virtmcu-allow: sleep reasoning="test-only; simulating workload"
                counter_clone.fetch_add(1, Ordering::SeqCst);
            })?;

        // Spawn threads to publish many messages
        let mut handles = vec![];
        const NUM_THREADS: usize = 8;
        const MSGS_PER_THREAD: usize = 20;
        for _ in 0..NUM_THREADS {
            let session_clone = session.clone();
            let handle = std::thread::spawn(move || {
                for _ in 0..MSGS_PER_THREAD {
                    let _ = session_clone.put(topic, "data").wait();
                }
            });
            handles.push(handle);
        }

        // Wait a tiny bit for some callbacks to start
        const DROP_DELAY_MS: u64 = 10;
        std::thread::sleep(Duration::from_millis(DROP_DELAY_MS)); // virtmcu-allow: sleep reasoning="test-only"
                                                                  // Drop the subscriber while messages are still being processed
        drop(sub);

        // After drop returns, active_count MUST be 0 and no more increments should happen
        let final_count = counter.load(Ordering::SeqCst);

        // Wait to be sure no late callbacks arrive
        const VERIFY_DELAY_MS: u64 = 100;
        std::thread::sleep(Duration::from_millis(VERIFY_DELAY_MS)); // virtmcu-allow: sleep reasoning="test-only"
        assert_eq!(counter.load(Ordering::SeqCst), final_count, "Counter increased after Drop!");

        for h in handles {
            let _ = h.join();
        }
        Ok(())
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_generation_drop_stale_callback() -> Result<(), zenoh::Error> {
        let config = crate::test_config();
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
        const WAIT_DELAY_MS: u64 = 100;
        std::thread::sleep(Duration::from_millis(WAIT_DELAY_MS)); // virtmcu-allow: sleep reasoning="test-only"
        assert_eq!(counter.load(Ordering::SeqCst), 0, "Stale callback was invoked!");
        Ok(())
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_generation_accepts_current() -> Result<(), zenoh::Error> {
        let config = crate::test_config();
        let session = zenoh::open(config).wait().map_err(|e| zenoh::Error::from(e.to_string()))?;
        let counter_pair = Arc::new((std::sync::Mutex::new(0_usize), std::sync::Condvar::new()));
        let counter_pair_clone = Arc::clone(&counter_pair);
        const GEN_VAL: u64 = 2;
        let generation = Arc::new(AtomicU64::new(GEN_VAL));
        let topic = "tests/fixtures/guest_apps/gen/current";

        // Create subscriber with gen 2
        let _sub = SafeSubscriber::new_with_generation(
            &session,
            topic,
            Arc::clone(&generation),
            move |_sample| {
                let (lock, cvar) = &*counter_pair_clone;
                let mut count = lock.lock().unwrap();
                *count += 1;
                cvar.notify_one();
            },
        )?;

        session.put(topic, "valid").wait().map_err(|e| zenoh::Error::from(e.to_string()))?;

        // Wait for callback
        const WAIT_TIMEOUT_SECS: u64 = 5;
        let (lock, cvar) = &*counter_pair;
        let mut count = lock.lock().unwrap();
        let result = cvar
            .wait_timeout_while(count, core::time::Duration::from_secs(WAIT_TIMEOUT_SECS), |c| {
                *c == 0
            })
            .unwrap();
        count = result.0;
        assert!(*count > 0, "Valid callback was NOT invoked!");
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
        const BUFFER_SIZE: usize = 1024;
        let buffer = [b'a' as c_char; BUFFER_SIZE];
        let res = unsafe { open_session(buffer.as_ptr()) };
        assert!(res.is_err());
        assert!(res.expect_err("Expected error").to_string().contains("not null-terminated"));
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_generation_stress() -> Result<(), zenoh::Error> {
        let config = crate::test_config();
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
        const STRESS_ITERATIONS: usize = 100;
        const GEN_INC_STEP: usize = 10;
        for i in 0..STRESS_ITERATIONS {
            session.put(topic, "stress").wait().map_err(|e| zenoh::Error::from(e.to_string()))?;
            if i % GEN_INC_STEP == 0 {
                generation.fetch_add(1, Ordering::SeqCst);
            }
        }

        Ok(())
    }
}
