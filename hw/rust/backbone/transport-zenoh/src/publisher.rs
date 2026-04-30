use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use crossbeam_channel::{bounded, Sender, TrySendError};
use std::thread::{self, JoinHandle};
use zenoh::pubsub::Publisher;
use zenoh::Wait;

/// A lock-free, fire-and-forget publisher wrapper to prevent blocking the BQL.
/// The `send` method pushes the payload to a bounded lock-free channel.
/// A background thread drains the channel and blocks on Zenoh operations.
/// If the channel is full, payloads are dropped to ensure QEMU is never blocked.
pub struct SafePublisher {
    tx: Sender<Vec<u8>>,
    is_valid: Arc<AtomicBool>,
    sender_thread: Option<JoinHandle<()>>,
    dropped_count: Arc<AtomicUsize>,
}

impl SafePublisher {
    /// Creates a new `SafePublisher` wrapping a Zenoh `Publisher`.
    pub fn new(publisher: Publisher<'static>) -> Self {
        let (tx, rx) = bounded::<Vec<u8>>(1024);
        let is_valid = Arc::new(AtomicBool::new(true));
        let valid_clone = Arc::clone(&is_valid);
        let dropped_count = Arc::new(AtomicUsize::new(0));

        let sender_thread = thread::spawn(move || {
            while let Ok(payload) = rx.recv() {
                if !valid_clone.load(Ordering::Acquire) {
                    break;
                }
                // We purposefully block the background thread during Zenoh's put.
                // The BQL thread (calling `send`) remains unblocked.
                if let Err(e) = publisher.put(payload).wait() {
                    virtmcu_qom::sim_err!("SafePublisher: put failed: {}", e);
                }
            }
        });

        Self { tx, is_valid, sender_thread: Some(sender_thread), dropped_count }
    }

    /// Dispatches a payload to the background sender thread.
    ///
    /// This call is non-blocking and safe to call while holding the BQL.
    pub fn send(&self, payload: Vec<u8>) {
        if self.is_valid.load(Ordering::Acquire) {
            if let Err(TrySendError::Full(_)) = self.tx.try_send(payload) {
                let dropped = self.dropped_count.fetch_add(1, Ordering::Relaxed);
                if dropped.is_multiple_of(100) {
                    virtmcu_qom::sim_warn!(
                        "SafePublisher queue full, dropped {} messages",
                        dropped + 1
                    );
                }
            }
        }
    }
}

/// A thread-safe, non-blocking Zenoh session-level publisher wrapper.
///
/// Unlike `SafePublisher`, this is not bound to a specific topic.
/// It takes a topic string for each `send` call.
pub struct SafeSessionPublisher {
    tx: Sender<(alloc::string::String, Vec<u8>)>,
    is_valid: Arc<AtomicBool>,
    sender_thread: Option<JoinHandle<()>>,
    dropped_count: Arc<AtomicUsize>,
}

impl SafeSessionPublisher {
    /// Creates a new `SafeSessionPublisher` wrapping a Zenoh `Session`.
    pub fn new(session: Arc<zenoh::Session>) -> Self {
        let (tx, rx) = bounded::<(alloc::string::String, Vec<u8>)>(1024);
        let is_valid = Arc::new(AtomicBool::new(true));
        let valid_clone = Arc::clone(&is_valid);
        let dropped_count = Arc::new(AtomicUsize::new(0));

        let sender_thread = thread::spawn(move || {
            while let Ok((topic, payload)) = rx.recv() {
                if !valid_clone.load(Ordering::Acquire) {
                    break;
                }
                // We purposefully block the background thread during Zenoh's put.
                // The BQL thread (calling `send`) remains unblocked.
                if let Err(e) = session.put(topic, payload).wait() {
                    virtmcu_qom::sim_err!("SafeSessionPublisher: put failed: {}", e);
                }
            }
        });

        Self { tx, is_valid, sender_thread: Some(sender_thread), dropped_count }
    }

    /// Dispatches a topic and payload to the background sender thread.
    ///
    /// This call is non-blocking and safe to call while holding the BQL.
    pub fn send(&self, topic: alloc::string::String, payload: Vec<u8>) {
        if self.is_valid.load(Ordering::Acquire) {
            if let Err(TrySendError::Full(_)) = self.tx.try_send((topic, payload)) {
                let dropped = self.dropped_count.fetch_add(1, Ordering::Relaxed);
                if dropped.is_multiple_of(100) {
                    virtmcu_qom::sim_warn!(
                        "SafeSessionPublisher queue full, dropped {} messages",
                        dropped + 1
                    );
                }
            }
        }
    }
}

impl Drop for SafePublisher {
    fn drop(&mut self) {
        self.is_valid.store(false, Ordering::Release);
        if let Some(handle) = self.sender_thread.take() {
            // Drop tx by replacing it with a new dummy channel, unblocking the background thread's `rx.recv()`
            drop(core::mem::replace(&mut self.tx, bounded(1).0));
            // We do a temporary BQL unlock if we are running in the main QEMU thread, to ensure we don't
            // deadlock against Zenoh calls that might also be blocked trying to re-acquire the BQL.
            let _unlock = virtmcu_qom::sync::Bql::temporary_unlock();
            let _ = handle.join();
        }
    }
}

impl Drop for SafeSessionPublisher {
    fn drop(&mut self) {
        self.is_valid.store(false, Ordering::Release);
        if let Some(handle) = self.sender_thread.take() {
            // Drop tx by replacing it with a new dummy channel, unblocking the background thread's `rx.recv()`
            drop(core::mem::replace(&mut self.tx, bounded(1).0));
            // We do a temporary BQL unlock if we are running in the main QEMU thread, to ensure we don't
            // deadlock against Zenoh calls that might also be blocked trying to re-acquire the BQL.
            let _unlock = virtmcu_qom::sync::Bql::temporary_unlock();
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::time::Duration;
    use std::time::Instant;
    use zenoh::Config;

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_safe_publisher_sends_payload() -> Result<(), zenoh::Error> {
        let config = Config::default();
        let session = zenoh::open(config).wait().map_err(|e| zenoh::Error::from(e.to_string()))?;
        let topic = "tests/fixtures/guest_apps/safe/pub/payload";

        let pub_ = session
            .declare_publisher(topic)
            .wait()
            .map_err(|e| zenoh::Error::from(e.to_string()))?;

        let safe_pub = SafePublisher::new(pub_);

        let sub = session
            .declare_subscriber(topic)
            .wait()
            .map_err(|e| zenoh::Error::from(e.to_string()))?;

        safe_pub.send(b"hello".to_vec());

        // Let background thread process and Zenoh router route.
        let msg = sub
            .recv_timeout(Duration::from_millis(50))
            .map_err(|e| zenoh::Error::from(e.to_string()))?
            .expect("No message received");
        assert_eq!(msg.payload().to_bytes().as_ref(), b"hello");

        drop(safe_pub);
        Ok(())
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_safe_publisher_non_blocking_under_load() -> Result<(), zenoh::Error> {
        let config = Config::default();
        let session = zenoh::open(config).wait().map_err(|e| zenoh::Error::from(e.to_string()))?;
        let topic = "tests/fixtures/guest_apps/safe/pub/load";

        let pub_ = session
            .declare_publisher(topic)
            .wait()
            .map_err(|e| zenoh::Error::from(e.to_string()))?;

        let safe_pub = SafePublisher::new(pub_);

        let start = Instant::now();
        for _ in 0..1000 {
            safe_pub.send(vec![1, 2, 3, 4, 5]);
        }
        let elapsed = start.elapsed();

        // Assert total wall-clock < 1ms for 1000 sends (proves non-blocking behavior)
        assert!(elapsed < Duration::from_millis(1), "send() blocked too long: {elapsed:?}");

        drop(safe_pub);
        Ok(())
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_safe_publisher_drop_joins_thread() -> Result<(), zenoh::Error> {
        let config = Config::default();
        let session = zenoh::open(config).wait().map_err(|e| zenoh::Error::from(e.to_string()))?;
        let topic = "tests/fixtures/guest_apps/safe/pub/drop";

        let pub_ = session
            .declare_publisher(topic)
            .wait()
            .map_err(|e| zenoh::Error::from(e.to_string()))?;

        let safe_pub = SafePublisher::new(pub_);
        safe_pub.send(b"hello".to_vec());

        let start = Instant::now();
        drop(safe_pub);
        let elapsed = start.elapsed();

        // Assert drop completes within 500ms
        assert!(elapsed < Duration::from_millis(500), "drop() took too long: {elapsed:?}");
        Ok(())
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_safe_publisher_drops_when_full() -> Result<(), zenoh::Error> {
        let config = Config::default();
        let session = zenoh::open(config).wait().map_err(|e| zenoh::Error::from(e.to_string()))?;
        let topic = "tests/fixtures/guest_apps/safe/pub/overflow";

        let pub_ = session
            .declare_publisher(topic)
            .wait()
            .map_err(|e| zenoh::Error::from(e.to_string()))?;

        let safe_pub = SafePublisher::new(pub_);

        let start = Instant::now();
        // Send 10,000 messages (exceeds the queue limit of 1024).
        // If it blocked, it would take much longer than 10ms.
        for _ in 0..10000 {
            safe_pub.send(vec![1, 2, 3]);
        }
        let elapsed = start.elapsed();

        // Should complete almost instantly because it drops packets
        assert!(elapsed < Duration::from_millis(10), "send() blocked on full queue: {elapsed:?}");

        // The background thread continues processing. Ensure drop also handles it cleanly.
        drop(safe_pub);
        Ok(())
    }
}
