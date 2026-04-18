#![allow(
    clippy::missing_safety_doc,
    clippy::collapsible_match,
    dead_code,
    unused_imports,
    clippy::len_zero
)]

use byteorder::{ByteOrder, LittleEndian};
use core::ffi::{c_char, c_void};
use std::ffi::CStr;
use std::ptr;

use zenoh::pubsub::Publisher;
use zenoh::pubsub::Subscriber;
use zenoh::Config;
use zenoh::Session;
use zenoh::Wait;

use virtmcu_api::ZenohFrameHeader;
use virtmcu_qom::chardev::*;
use virtmcu_qom::sync::*;
use virtmcu_qom::timer::*;

struct RxFrame {
    delivery_vtime: u64,
    data: Vec<u8>,
}

pub struct ZenohChardevState {
    chr: *mut Chardev,
    #[allow(dead_code)]
    session: Session,
    publisher: Publisher<'static>,
    #[allow(dead_code)]
    subscriber: Subscriber<()>,
    queue: std::sync::Mutex<Vec<RxFrame>>,
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_chardev_init(
    node_id: u32,
    router: *const c_char,
    chr: *mut Chardev,
) -> *mut ZenohChardevState {
    let session = match virtmcu_zenoh::open_session(router) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "[zenoh-chardev] node={}: FAILED to open Zenoh session: {}",
                node_id, e
            );
            return ptr::null_mut();
        }
    };

    let tx_topic = format!("virtmcu/uart/{}/tx", node_id);
    let rx_topic = format!("virtmcu/uart/{}/rx", node_id);

    let publisher = session.declare_publisher(tx_topic).wait().unwrap();
    let queue = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));

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

    Box::into_raw(Box::new(
        ZenohChardevState {
            chr,
            session,
            publisher,
            subscriber,
            queue: std::sync::Mutex::new(Vec::new()),
        }
        .set_queue(queue),
    ))
}

impl ZenohChardevState {
    fn set_queue(mut self, q: std::sync::Arc<std::sync::Mutex<Vec<RxFrame>>>) -> Self {
        self.queue = std::sync::Mutex::new(
            std::sync::Arc::try_unwrap(q)
                .ok()
                .unwrap()
                .into_inner()
                .unwrap(),
        );
        self
    }
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_chardev_write(
    backend: *mut ZenohChardevState,
    buf: *const u8,
    size: usize,
) {
    let b = &*backend;
    let vtime = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) as u64;

    let mut payload = Vec::with_capacity(12 + size);
    let mut header = [0u8; 12];
    LittleEndian::write_u64(&mut header[0..8], vtime);
    LittleEndian::write_u32(&mut header[8..12], size as u32);
    payload.extend_from_slice(&header);
    payload.extend_from_slice(std::slice::from_raw_parts(buf, size));

    let _ = b.publisher.put(payload).wait();
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_chardev_poll(backend: *mut ZenohChardevState) {
    let b = &*backend;
    let vtime = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) as u64;
    let mut q = b.queue.lock().unwrap();

    let mut i = 0;
    while i < q.len() {
        if q[i].delivery_vtime <= vtime {
            let frame = q.remove(i);
            qemu_chr_be_write(b.chr, frame.data.as_ptr(), frame.data.len());
            // Don't increment i
        } else {
            i += 1;
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_chardev_free(backend: *mut ZenohChardevState) {
    if !backend.is_null() {
        drop(Box::from_raw(backend));
    }
}
