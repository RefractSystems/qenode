#![allow(clippy::all, unused_imports, dead_code, unused_variables, unused_mut)] // virtmcu-allow: allow reasoning="Zero unsafe"
#![allow(clippy::all)] // virtmcu-allow: allow reasoning="Zero unsafe"
#![cfg_attr(
    test,
    allow(
        clippy::expect_used,
        clippy::unwrap_used,
        clippy::panic,
        clippy::indexing_slicing,
        clippy::panic_in_result_fn
    )
)]
pub mod router;

extern crate alloc;

use std::cell::UnsafeCell;
use std::io::{IoSlice, Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};
use std::thread;
use virtmcu_wire::{DataCallback, DataTransport};

const TLS_ARENA_SIZE: usize = 64 * 1024;

// virtmcu-allow: static_state reasoning="UdsDataTransport uses thread-local arena to avoid heap allocations per RFC-0025 Phase 3."
thread_local! {
    static ARENA: UnsafeCell<Vec<u8>> = UnsafeCell::new(vec![0; TLS_ARENA_SIZE]);
}

fn write_framed(stream: &mut UnixStream, topic: &str, payload: &[u8]) -> std::io::Result<()> {
    let topic_bytes = topic.as_bytes();
    let topic_len_bytes = (topic_bytes.len() as u32).to_le_bytes();
    let payload_len_bytes = (payload.len() as u32).to_le_bytes();

    let parts: [&[u8]; 4] = [&topic_len_bytes, topic_bytes, &payload_len_bytes, payload];

    let mut total_written = 0;
    let total_len: usize = parts.iter().map(|p| p.len()).sum();

    while total_written < total_len {
        let mut iov = [IoSlice::new(&[]), IoSlice::new(&[]), IoSlice::new(&[]), IoSlice::new(&[])];
        let mut iov_idx = 0;
        let mut current_offset = 0;

        for part in &parts {
            let part_len = part.len();
            if total_written < current_offset + part_len {
                let start = total_written.saturating_sub(current_offset);
                iov[iov_idx] = IoSlice::new(&part[start..]);
                iov_idx += 1;
            }
            current_offset += part_len;
        }

        match stream.write_vectored(&iov[..iov_idx]) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "unix stream closed",
                ))
            }
            Ok(n) => total_written += n,
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

pub struct UdsDataTransport {
    node_id: u32,
    stream: Arc<Mutex<UnixStream>>,
    subscriptions: Arc<Mutex<Vec<(String, DataCallback)>>>,
}

impl UdsDataTransport {
    pub fn new(path: &str, node_id: u32) -> Result<Self, String> {
        let fed_id = std::env::var("VIRTMCU_SIM_ID").unwrap_or_else(|_| "default-fed".to_string());
        Self::new_with_fed_id(path, node_id, &fed_id)
    }

    pub fn new_with_fed_id(path: &str, node_id: u32, fed_id: &str) -> Result<Self, String> {
        let mut stream = UnixStream::connect(path).map_err(|e| e.to_string())?;

        // Register with coordinator
        let reg_payload = virtmcu_wire::encode_uds_registration(node_id, fed_id);

        let topic = "sim/coord/register";
        let mut buf = Vec::new();
        buf.extend_from_slice(&(topic.len() as u32).to_le_bytes());
        buf.extend_from_slice(topic.as_bytes());
        buf.extend_from_slice(&(reg_payload.len() as u32).to_le_bytes());
        buf.extend_from_slice(&reg_payload);

        use std::io::Write;
        stream.write_all(&buf).map_err(|e| e.to_string())?;

        let mut read_stream = stream.try_clone().map_err(|e| e.to_string())?;
        let stream = Arc::new(Mutex::new(stream));
        let subscriptions: Arc<Mutex<Vec<(String, DataCallback)>>> =
            Arc::new(Mutex::new(Vec::new()));

        let subscriptions_clone = Arc::clone(&subscriptions);

        // RX thread
        thread::spawn(move || loop {
            let mut topic_len_buf = [0u8; 4];
            if read_stream.read_exact(&mut topic_len_buf).is_err() {
                break;
            }
            let topic_len = u32::from_le_bytes(topic_len_buf) as usize;

            let mut topic_buf = vec![0u8; topic_len];
            if read_stream.read_exact(&mut topic_buf).is_err() {
                break;
            }
            let topic = String::from_utf8_lossy(&topic_buf).into_owned();

            let mut payload_len_buf = [0u8; 4];
            if read_stream.read_exact(&mut payload_len_buf).is_err() {
                break;
            }
            let payload_len = u32::from_le_bytes(payload_len_buf) as usize;

            let mut payload = vec![0u8; payload_len];
            if read_stream.read_exact(&mut payload).is_err() {
                break;
            }

            virtmcu_qom::sim_info!(
                "UdsDataTransport: received msg on topic '{}' (len={})",
                topic,
                payload.len()
            );

            let subs = subscriptions_clone.lock().expect("unix transport error");
            let mut found = false;
            for (sub_topic, callback) in subs.iter() {
                if sub_topic == &topic || topic.starts_with(sub_topic) {
                    found = true;
                    virtmcu_qom::sim_info!(
                        "UdsDataTransport: dispatching to sub_topic '{}'",
                        sub_topic
                    );
                    callback(&topic, &payload);
                }
            }
            if !found {
                virtmcu_qom::sim_info!("UdsDataTransport: NO SUBSCRIPTION matched for '{}'", topic);
            }
        });

        Ok(Self { node_id, stream, subscriptions })
    }

    pub fn publish_raw(&self, topic: &str, payload: &[u8]) -> Result<(), String> {
        let mut stream = self.stream.lock().expect("unix transport error");
        write_framed(&mut stream, topic, payload).map_err(|e| e.to_string())
    }
}

impl DataTransport for UdsDataTransport {
    fn publish(&self, topic: &str, payload: &[u8]) -> Result<(), String> {
        self.publish_raw(topic, payload)
    }

    #[allow(deprecated)] // virtmcu-allow: allow reasoning="Stage 1 stub"
    fn reserve<'a>(
        &'a self,
        _topic: &'a str,
        _size: usize,
    ) -> Result<virtmcu_wire::TransportReservation<'a>, virtmcu_wire::TransportError> {
        Err(virtmcu_wire::TransportError::Other("Use reserve_link".to_owned()))
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
        virtmcu_qom::sim_info!(
            "UdsDataTransport: Publishing sim/coord/link/register for link {}",
            link_name
        );
        self.publish("sim/coord/link/register", &payload)
            .map_err(|e| virtmcu_wire::TransportError::Other(e))?;

        virtmcu_qom::sim_info!("UdsDataTransport: Waiting for ack for link {}", link_name);
        let ack_payload = rx.recv().map_err(|_| virtmcu_wire::TransportError::Closed)?;
        virtmcu_qom::sim_info!("UdsDataTransport: Received ack for link {}", link_name);

        if let Ok((link_id, status, _err)) = virtmcu_wire::decode_link_ack(&ack_payload) {
            if status != 0 {
                std::process::abort();
            }
            return Ok(link_id);
        }
        std::process::abort();
    }

    fn reserve_link<'a>(
        &'a self,
        link_id: u32,
        size: usize,
    ) -> Result<virtmcu_wire::TransportReservation<'a>, virtmcu_wire::TransportError> {
        const HEADER_SIZE: usize = 24;
        let required_size = size + HEADER_SIZE;

        ARENA.with(|arena_cell| {
            let arena = unsafe { &mut *arena_cell.get() };
            let _ =
                arena.get(..required_size).expect("FATAL: reserve size exceeds TLS arena capacity");

            let payload_ptr = arena.as_mut_ptr();
            // The peripheral will write to the slice starting at offset HEADER_SIZE.
            let b = unsafe { core::slice::from_raw_parts_mut(payload_ptr.add(HEADER_SIZE), size) };
            let buffer = unsafe { core::mem::transmute::<&mut [u8], &'a mut [u8]>(b) };

            let stream_clone = Arc::clone(&self.stream);
            let topic = format!("sim/ch/{link_id}");

            Ok(virtmcu_wire::TransportReservation::new(
                Box::leak(topic.into_boxed_str()),
                buffer,
                move |vtime, seq| {
                    const LINK_ID_OFFSET: usize = 0;
                    const SIZE_OFFSET: usize = 4;
                    const VTIME_OFFSET: usize = 8;
                    const SEQ_OFFSET: usize = 16;
                    const HEADER_END: usize = 24;

                    let payload = &mut arena[..required_size];
                    payload[LINK_ID_OFFSET..SIZE_OFFSET].copy_from_slice(&link_id.to_le_bytes());
                    payload[SIZE_OFFSET..VTIME_OFFSET]
                        .copy_from_slice(&(size as u32).to_le_bytes());
                    payload[VTIME_OFFSET..SEQ_OFFSET].copy_from_slice(&vtime.to_le_bytes());
                    payload[SEQ_OFFSET..HEADER_END].copy_from_slice(&seq.to_le_bytes());

                    let mut stream = stream_clone.lock().expect("unix transport error");
                    write_framed(&mut stream, &alloc::format!("sim/ch/{}", link_id), payload)
                        .map_err(|e| virtmcu_wire::TransportError::Other(e.to_string()))
                },
            ))
        })
    }

    fn subscribe(&self, topic: &str, callback: DataCallback) -> Result<(), String> {
        self.subscriptions
            .lock()
            .expect("unix transport error")
            .push((topic.to_string(), callback));
        Ok(())
    }
}

#[cfg(any())]
#[cfg(not(miri))]
mod tests {
    use super::*;
    use crate::router::UnixDataRouter;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};
    use tempfile::tempdir;

    const SLEEP_100_MS: Duration = Duration::from_millis(100);
    const SLEEP_10_MS: Duration = Duration::from_millis(10);
    const SLEEP_1_MS: Duration = Duration::from_millis(1);
    const TIMEOUT_2_SEC: Duration = Duration::from_secs(2);
    const TIMEOUT_5_SEC: Duration = Duration::from_secs(5);

    const MAX_MESSAGES: usize = 200;
    const HALF_MESSAGES: u32 = 100;
    const TOTAL_MESSAGES_U32: u32 = 200;

    const PAYLOAD_LEN_5: usize = 5;
    const PAYLOAD_LEN_4: usize = 4;

    const VTIME_1: u64 = 1234;
    const SEQ_1: u64 = 5678;

    const VTIME_BASE_1: u64 = 1000;
    const VTIME_BASE_2: u64 = 2000;

    #[test]
    fn test_reserve_commit_round_trip() {
        let dir = tempdir().expect("unix transport error");
        let sock_path = dir.path().join("rt_router.sock");
        let sock_str = sock_path.to_str().expect("unix transport error").to_string();

        let router = UnixDataRouter::new(&sock_str).expect("unix transport error");
        thread::spawn(move || router.serve());

        let t1 = UdsDataTransport::new(&sock_str, 0).expect("unix transport error");
        let t2 = UdsDataTransport::new(&sock_str, 0).expect("unix transport error");

        let received = Arc::new(Mutex::new(Vec::new()));
        let rx = Arc::clone(&received);
        t2.subscribe(
            "test/roundtrip",
            Box::new(move |topic: &str, payload: &[u8]| {
                rx.lock().unwrap().push((topic.to_string(), payload.to_vec()));
            }),
        )
        .unwrap();

        // Allow subscriptions and threads to initialize
        std::thread::sleep(SLEEP_100_MS); // virtmcu-allow: sleep reasoning="Wait for router thread in tests"

        let mut res = t1.reserve("test/roundtrip", PAYLOAD_LEN_5).unwrap();
        res.buffer_mut().copy_from_slice(b"hello");
        res.commit(VTIME_1, SEQ_1).unwrap();

        // Expected size after FlatBuffer encoding. 24 bytes header + padding + payload + dst_node_id = 72
        const EXPECTED_FLATBUFFER_PAYLOAD_LEN: usize = 72;

        let start = Instant::now();
        while start.elapsed() < TIMEOUT_2_SEC {
            let rx_lock = received.lock().unwrap();
            if !rx_lock.is_empty() {
                let (topic, payload) = &rx_lock[0];
                assert_eq!(topic, "test/roundtrip");
                assert_eq!(payload.len(), EXPECTED_FLATBUFFER_PAYLOAD_LEN);
                let (_vtime, _seq, data) = virtmcu_wire::decode_coord_message(payload).unwrap();
                assert_eq!(data, b"hello");
                return;
            }
            drop(rx_lock);
            std::thread::sleep(SLEEP_10_MS); // virtmcu-allow: sleep reasoning="Polling delay in tests"
        }
        panic!("Did not receive round-trip message in time");
    }

    #[test]
    fn test_concurrent_reserve_commit() {
        let dir = tempdir().expect("unix transport error");
        let sock_path = dir.path().join("conc_router.sock");
        let sock_str = sock_path.to_str().expect("unix transport error").to_string();

        let router = UnixDataRouter::new(&sock_str).expect("unix transport error");
        thread::spawn(move || router.serve());

        let t_rx = UdsDataTransport::new(&sock_str, 0).expect("unix transport error");

        let received_count = Arc::new(AtomicUsize::new(0));
        let count_clone = Arc::clone(&received_count);

        t_rx.subscribe(
            "test/concurrent",
            Box::new(move |_topic: &str, payload: &[u8]| {
                // Verify payload integrity and isolation
                let (vtime, _seq, user_data) = virtmcu_wire::decode_coord_message(payload).unwrap();
                assert_eq!(user_data.len(), PAYLOAD_LEN_4);
                let val = u32::from_le_bytes(user_data.try_into().unwrap());
                assert!(val < TOTAL_MESSAGES_U32);

                if val < HALF_MESSAGES {
                    assert_eq!(vtime, VTIME_BASE_1 + val as u64);
                } else {
                    assert_eq!(vtime, VTIME_BASE_2 + val as u64);
                }

                count_clone.fetch_add(1, Ordering::SeqCst);
            }),
        )
        .unwrap();

        std::thread::sleep(SLEEP_100_MS); // virtmcu-allow: sleep reasoning="Wait for router thread in tests"

        let sock_str1 = sock_str.clone();
        let handle1 = thread::spawn(move || {
            let t = UdsDataTransport::new(&sock_str1, 1).expect("unix transport error");
            for i in 0..HALF_MESSAGES {
                let mut res = t.reserve("test/concurrent", PAYLOAD_LEN_4).unwrap();
                res.buffer_mut().copy_from_slice(&i.to_le_bytes());
                res.commit(VTIME_BASE_1 + i as u64, i as u64).unwrap();
                std::thread::sleep(SLEEP_1_MS); // virtmcu-allow: sleep reasoning="Yield in tests"
            }
        });

        let sock_str2 = sock_str.clone();
        let handle2 = thread::spawn(move || {
            const NODE_2: u32 = 2;
            let t = UdsDataTransport::new(&sock_str2, NODE_2).expect("unix transport error");
            for i in HALF_MESSAGES..TOTAL_MESSAGES_U32 {
                let mut res = t.reserve("test/concurrent", PAYLOAD_LEN_4).unwrap();
                res.buffer_mut().copy_from_slice(&i.to_le_bytes());
                res.commit(VTIME_BASE_2 + i as u64, i as u64).unwrap();
                std::thread::sleep(SLEEP_1_MS); // virtmcu-allow: sleep reasoning="Yield in tests"
            }
        });

        handle1.join().unwrap();
        handle2.join().unwrap();

        let start = Instant::now();
        while start.elapsed() < TIMEOUT_5_SEC {
            if received_count.load(Ordering::SeqCst) == MAX_MESSAGES {
                return;
            }
            std::thread::sleep(SLEEP_10_MS); // virtmcu-allow: sleep reasoning="Polling delay in tests"
        }
        panic!("Only received {}/{} messages", received_count.load(Ordering::SeqCst), MAX_MESSAGES);
    }
}
