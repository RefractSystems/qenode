#![allow(
    clippy::missing_safety_doc,
    clippy::collapsible_match,
    dead_code,
    unused_imports,
    clippy::len_zero
)]
extern crate libc;

use byteorder::{ByteOrder, LittleEndian};
use core::ffi::{c_char, c_void};
use std::collections::BinaryHeap;
use std::ffi::CStr;
use std::ptr;
use zenoh::pubsub::{Publisher, Subscriber};
use zenoh::{Config, Session, Wait};

use virtmcu_qom::sync::*;
use virtmcu_qom::timer::*;

#[repr(C)]
#[derive(Copy, Clone)]
struct ZenohFrameHeader {
    delivery_vtime_ns: u64,
    size: u32,
}

#[derive(Eq, PartialEq, Debug)]
struct RxFrame {
    delivery_vtime: u64,
    data: Vec<u8>,
}

// Implement Ord such that SMALLER vtime has HIGHER priority in BinaryHeap (which is a max-heap)
impl Ord for RxFrame {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Reverse comparison for min-heap behavior in BinaryHeap
        other.delivery_vtime.cmp(&self.delivery_vtime)
    }
}

impl PartialOrd for RxFrame {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

pub struct ZenohNetdevBackend {
    session: Session,
    publisher: Publisher<'static>,
    subscriber: Option<Subscriber<()>>,
    node_id: u32,
    nc: *mut c_void,
    rx_timer: *mut QemuTimer,
    // Max-heap of RxFrame, where "greater" means smaller vtime (effectively a min-heap of vtime)
    rx_queue: std::sync::Mutex<BinaryHeap<RxFrame>>,
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_netdev_init(
    nc: *mut c_void,
    node_id: u32,
    router: *const c_char,
    topic: *const c_char,
) -> *mut ZenohNetdevBackend {
    let session = match virtmcu_zenoh::open_session(router) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "[zenoh-netdev] node={}: FAILED to open Zenoh session: {}",
                node_id, e
            );
            return ptr::null_mut();
        }
    };

    let topic_tx;
    let topic_rx;
    if !topic.is_null() {
        let t = CStr::from_ptr(topic).to_str().unwrap_or("");
        topic_tx = format!("{}/tx", t);
        topic_rx = format!("{}/rx", t);
    } else {
        topic_tx = format!("sim/eth/frame/{}/tx", node_id);
        topic_rx = format!("sim/eth/frame/{}/rx", node_id);
    }

    let publisher = session.declare_publisher(topic_tx).wait().unwrap();

    // Two-phase init: allocate first to get a stable address for the callback.
    // Use std::mem::MaybeUninit to represent the partially-initialized state safely.
    let backend_ptr_raw = Box::into_raw(Box::new(ZenohNetdevBackend {
        session: session.clone(),
        publisher,
        subscriber: None,
        node_id,
        nc,
        rx_timer: ptr::null_mut(),
        rx_queue: std::sync::Mutex::new(BinaryHeap::with_capacity(1024)),
    }));
    let backend_ptr_usize = backend_ptr_raw as usize;

    let rx_timer = virtmcu_timer_new_ns(
        QEMU_CLOCK_VIRTUAL,
        rx_timer_cb,
        backend_ptr_raw as *mut c_void,
    );

    // Update timer before starting subscriber to avoid null-deref in callback
    (*backend_ptr_raw).rx_timer = rx_timer;

    let subscriber = session
        .declare_subscriber(topic_rx)
        .callback(move |sample| {
            let backend = &*(backend_ptr_usize as *const ZenohNetdevBackend);
            on_rx_frame(backend, sample);
        })
        .wait()
        .unwrap();

    (*backend_ptr_raw).subscriber = Some(subscriber);

    backend_ptr_raw
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_netdev_receive_rust(
    backend: *mut ZenohNetdevBackend,
    buf: *const u8,
    size: usize,
) -> isize {
    if backend.is_null() || buf.is_null() {
        return -1;
    }
    let b = &*backend;

    let vtime = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL);

    let mut msg = Vec::with_capacity(12 + size);
    let mut hdr_bytes = [0u8; 12];
    LittleEndian::write_u64(&mut hdr_bytes[0..8], vtime as u64);
    LittleEndian::write_u32(&mut hdr_bytes[8..12], size as u32);

    msg.extend_from_slice(&hdr_bytes);
    msg.extend_from_slice(std::slice::from_raw_parts(buf, size));

    let _ = b.publisher.put(msg).wait();
    size as isize
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_netdev_cleanup_rust(backend: *mut ZenohNetdevBackend) {
    if backend.is_null() {
        return;
    }
    let b = Box::from_raw(backend);
    if !b.rx_timer.is_null() {
        // Cancel and free the timer first to prevent use-after-free
        virtmcu_timer_del(b.rx_timer);
        virtmcu_timer_free(b.rx_timer);
    }
    // subscriber and session will be dropped automatically when b is dropped
}

fn on_rx_frame(backend: &ZenohNetdevBackend, sample: zenoh::sample::Sample) {
    let payload = sample.payload();
    if payload.len() < 12 {
        return;
    }

    let bytes = payload.to_bytes();
    let vtime = LittleEndian::read_u64(&bytes[0..8]);
    let size = LittleEndian::read_u32(&bytes[8..12]) as usize;

    if size > 1024 * 1024 || bytes.len() < 12 + size {
        return;
    }

    let frame_data = bytes[12..12 + size].to_vec();

    // CRITICAL: Acquire BQL before modifying QEMU timer state or taking internal locks
    // to prevent AB-BA deadlocks with the QEMU main thread.
    let _bql_guard = virtmcu_qom::sync::Bql::lock();

    let mut queue = backend.rx_queue.lock().unwrap();
    if queue.len() >= 1024 {
        eprintln!(
            "[zenoh-netdev] RX queue overflow on node {}, dropping earliest frame",
            backend.node_id
        );
        queue.pop();
    }

    queue.push(RxFrame {
        delivery_vtime: vtime,
        data: frame_data,
    });

    if let Some(earliest) = queue.peek() {
        // Mod timer for the earliest frame.
        if !backend.rx_timer.is_null() {
            unsafe {
                virtmcu_timer_mod(backend.rx_timer, earliest.delivery_vtime as i64);
            }
        }
    }
}

extern "C" fn rx_timer_cb(opaque: *mut c_void) {
    let backend = unsafe { &*(opaque as *mut ZenohNetdevBackend) };

    loop {
        let frame = {
            let mut queue = backend.rx_queue.lock().unwrap();
            let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;

            match queue.peek() {
                Some(earliest) if earliest.delivery_vtime <= now => queue.pop().unwrap(),
                Some(earliest) => {
                    // Re-arm for the next earliest frame
                    unsafe {
                        virtmcu_timer_mod(backend.rx_timer, earliest.delivery_vtime as i64);
                    }
                    return;
                }
                None => return,
            }
        };

        // Assert determinism: frame must not be delivered before its virtual time.
        debug_assert!(
            frame.delivery_vtime <= unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64,
            "zenoh-netdev: frame delivered before its vtime (timer fired too early)"
        );

        // Emit a log line that the determinism test can parse to verify vtime ordering.
        eprintln!(
            "[virtmcu-netdev] RX deliver node={} vtime={} size={}",
            backend.node_id,
            frame.delivery_vtime,
            frame.data.len()
        );

        unsafe {
            virtmcu_qom::net::qemu_send_packet(
                backend.nc as *mut _,
                frame.data.as_ptr(),
                frame.data.len(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rx_frame_ordering() {
        let mut heap = BinaryHeap::new();
        heap.push(RxFrame {
            delivery_vtime: 200,
            data: vec![1],
        });
        heap.push(RxFrame {
            delivery_vtime: 100,
            data: vec![2],
        });
        heap.push(RxFrame {
            delivery_vtime: 150,
            data: vec![3],
        });

        assert_eq!(heap.pop().unwrap().delivery_vtime, 100);
        assert_eq!(heap.pop().unwrap().delivery_vtime, 150);
        assert_eq!(heap.pop().unwrap().delivery_vtime, 200);
    }
}
