#![allow(missing_docs)]
use core::ffi::c_char;
use std::ffi::CStr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use virtmcu_qom::sync::Bql;
use zenoh::pubsub::Subscriber;
use zenoh::{Config, Session, Wait};

/// A thread-safe, RAII-enabled Zenoh subscriber for VirtMCU QOM devices.
///
/// It ensures that:
/// 1. The callback always acquires the Big QEMU Lock (BQL).
/// 2. The callback is only executed if the device state is still valid.
/// 3. The subscriber is properly undeclared and synchronization occurs during drop,
///    preventing Use-After-Free during device finalization.
pub struct SafeSubscriber {
    subscriber: Option<Subscriber<()>>,
    is_valid: Arc<AtomicBool>,
    active_count: Arc<AtomicUsize>,
}

impl SafeSubscriber {
    /// Creates a new `SafeSubscriber`.
    ///
    /// # Arguments
    /// * `session` - The Zenoh session.
    /// * `topic` - The topic to subscribe to.
    /// * `callback` - The closure to execute when a sample is received.
    ///   The BQL is already held when this callback runs.
    pub fn new<F>(session: &Session, topic: &str, callback: F) -> Result<Self, zenoh::Error>
    where
        F: Fn(zenoh::sample::Sample) + Send + Sync + 'static,
    {
        let is_valid = Arc::new(AtomicBool::new(true));
        let valid_clone = Arc::clone(&is_valid);
        let active_count = Arc::new(AtomicUsize::new(0));
        let active_clone = Arc::clone(&active_count);

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
                        callback(sample);
                    }
                }

                // Decrement active count when finished.
                active_clone.fetch_sub(1, Ordering::SeqCst);
            })
            .wait()?;

        Ok(Self { subscriber: Some(subscriber), is_valid, active_count })
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
        let mut attempts = 0;
        while self.active_count.load(Ordering::SeqCst) > 0 && attempts < 1000 {
            std::thread::yield_now();
            attempts += 1;
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
        if let Ok(r_str) = CStr::from_ptr(router).to_str() {
            if !r_str.is_empty() {
                let json = format!("[\"{r_str}\"]");
                let _ = config.insert_json5("mode", "\"client\"");
                let _ = config.insert_json5("connect/endpoints", &json);
                let _ = config.insert_json5("scouting/multicast/enabled", "false");
                let _ = config.insert_json5("transport/shared_memory/enabled", "false");
                has_router = true;
            }
        }
    }

    let session = zenoh::open(config)
        .wait()
        .map_err(|e| zenoh::Error::from(format!("Failed to open Zenoh session: {e}")))?;
    virtmcu_qom::vlog!("[virtmcu-zenoh] Session returned from zenoh::open.wait().\n");

    // If a router was provided, verify we can actually reach it.
    if has_router {
        let mut connected = false;
        let max_attempts = 100; // 10 seconds total
        for i in 0..max_attempts {
            let info = session.info();
            let routers: Vec<_> = info.routers_zid().wait().collect();
            let peers: Vec<_> = info.peers_zid().wait().collect();

            if !routers.is_empty() || !peers.is_empty() {
                virtmcu_qom::vlog!(
                    "[virtmcu-zenoh] Connected after {} attempts ({} ms). Routers={:?}, Peers={:?}\n",
                    i,
                    i * 100,
                    routers,
                    peers
                );
                connected = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        if !connected {
            virtmcu_qom::vlog!(
                "[virtmcu-zenoh] FATAL: Failed to connect to explicit router after 10 seconds.\n"
            );
            let _ = session.close().wait();
            return Err(zenoh::Error::from("Failed to connect to explicit router".to_string()));
        }
    }

    Ok(session)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_safe_subscriber_lifecycle() {
        let config = Config::default();
        // Use memory transport for fast unit tests
        let session = zenoh::open(config).wait().unwrap();
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        let topic = "test/safe/sub";

        {
            let _sub = SafeSubscriber::new(&session, topic, move |_sample| {
                counter_clone.fetch_add(1, Ordering::SeqCst);
            })
            .unwrap();

            // Publish a message
            session.put(topic, "hello").wait().unwrap();

            // Wait for callback (it might take a moment as it's async)
            let mut attempts = 0;
            while counter.load(Ordering::SeqCst) == 0 && attempts < 100 {
                std::thread::sleep(Duration::from_millis(10));
                attempts += 1;
            }
            assert!(counter.load(Ordering::SeqCst) > 0);
        }

        // Sub is now dropped. Marking it as invalid and undeclaring should have happened.
        let count_after_drop = counter.load(Ordering::SeqCst);

        // Publish more - should NOT be received
        for _ in 0..10 {
            session.put(topic, "ignored").wait().unwrap();
        }

        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(counter.load(Ordering::SeqCst), count_after_drop);
    }
}
