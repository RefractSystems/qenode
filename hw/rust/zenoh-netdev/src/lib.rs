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

use virtmcu_api::ZenohFrameHeader;
use virtmcu_qom::sync::*;
use virtmcu_qom::timer::*;

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

#[repr(C)]
pub struct NetClientInfo {
    pub type_: i32,
    pub size: usize,
    pub receive: Option<unsafe extern "C" fn(nc: *mut c_void, buf: *const u8, size: usize) -> isize>,
    pub cleanup: Option<unsafe extern "C" fn(nc: *mut c_void)>,
}

pub struct ZenohNetdevState {
    nc: *mut c_void,
    session: Session,
    publisher: Publisher<'static>,
    #[allow(dead_code)]
    subscriber: Subscriber<()>,
    queue: std::sync::Mutex<BinaryHeap<RxFrame>>,
    node_id: u32,
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_netdev_init(
    node_id: u32,
    router: *const c_char,
    nc: *mut c_void,
) -> *mut ZenohNetdevState {
    let session = match virtmcu_zenoh::open_session(router) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[zenoh-netdev] node={}: FAILED to open Zenoh session: {}", node_id, e);
            return ptr::null_mut();
        }
    };

    let tx_topic = format!("sim/net/raw/{}", node_id);
    let rx_topic = format!("sim/net/raw/{}", node_id);

    let publisher = session.declare_publisher(tx_topic).wait().unwrap();
    let queue = std::sync::Arc::new(std::sync::Mutex::new(BinaryHeap::new()));

    let queue_clone = queue.clone();
    let subscriber = session
        .declare_subscriber(rx_topic)
        .callback(move |sample| {
            let payload = sample.payload().to_bytes();
            if payload.len() < 12 {
                return;
            }
            let header_bytes = &payload[0..12];
            let delivery_vtime = LittleEndian::read_u64(&header_bytes[0..8]);
            let size = LittleEndian::read_u32(&header_bytes[8..12]) as usize;

            if payload.len() < 12 + size {
                return;
            }

            let data = payload[12..12 + size].to_vec();
            let mut q = queue_clone.lock().unwrap();
            q.push(RxFrame {
                delivery_vtime,
                data,
            });
        })
        .wait()
        .unwrap();

    Box::into_raw(Box::new(ZenohNetdevState {
        nc,
        session,
        publisher,
        subscriber,
        queue: std::sync::Mutex::new(BinaryHeap::new()),
        node_id,
    }.set_queue(queue)))
}

impl ZenohNetdevState {
    fn set_queue(mut self, q: std::sync::Arc<std::sync::Mutex<BinaryHeap<RxFrame>>>) -> Self {
        self.queue = std::sync::Mutex::new(std::sync::Arc::try_unwrap(q).ok().unwrap().into_inner().unwrap());
        self
    }
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_netdev_receive(
    backend: *mut ZenohNetdevState,
    buf: *const u8,
    size: usize,
) -> isize {
    let b = &*backend;
    let vtime = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) as u64;

    let mut payload = Vec::with_capacity(12 + size);
    let mut header = [0u8; 12];
    LittleEndian::write_u64(&mut header[0..8], vtime);
    LittleEndian::write_u32(&mut header[8..12], size as u32);
    payload.extend_from_slice(&header);
    payload.extend_from_slice(std::slice::from_raw_parts(buf, size));

    let _ = b.publisher.put(payload).wait();
    size as isize
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_netdev_can_receive(backend: *mut ZenohNetdevState) -> bool {
    let b = &*backend;
    let vtime = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) as u64;
    let q = b.queue.lock().unwrap();
    if let Some(frame) = q.peek() {
        return frame.delivery_vtime <= vtime;
    }
    false
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_netdev_read(
    backend: *mut ZenohNetdevState,
    buf: *mut u8,
    max_size: usize,
) -> isize {
    let b = &*backend;
    let vtime = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) as u64;
    let mut q = b.queue.lock().unwrap();

    if let Some(frame) = q.peek() {
        if frame.delivery_vtime <= vtime {
            let frame = q.pop().unwrap();
            let size = frame.data.len().min(max_size);
            ptr::copy_nonoverlapping(frame.data.as_ptr(), buf, size);
            return size as isize;
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_netdev_free(backend: *mut ZenohNetdevState) {
    if !backend.is_null() {
        drop(Box::from_raw(backend));
    }
}
