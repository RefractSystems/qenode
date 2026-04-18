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

struct RxFrame {
    delivery_vtime: u64,
    data: Vec<u8>,
}

pub struct ZenohNetdevBackend {
    session: Session,
    publisher: Publisher<'static>,
    subscriber: Subscriber<()>,
    node_id: u32,
    nc: *mut c_void,
    rx_timer: *mut QemuTimer,
    rx_queue: std::sync::Mutex<Vec<RxFrame>>,
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
        Err(_) => return ptr::null_mut(),
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

    let backend_ptr_raw = Box::into_raw(Box::new(std::mem::MaybeUninit::<ZenohNetdevBackend>::uninit())) as *mut ZenohNetdevBackend;
    let backend_ptr_usize = backend_ptr_raw as usize;

    let subscriber = session
        .declare_subscriber(topic_rx)
        .callback(move |sample| {
            let backend = &*(backend_ptr_usize as *const ZenohNetdevBackend);
            on_rx_frame(backend, sample);
        })
        .wait()
        .unwrap();

    let rx_timer = virtmcu_timer_new_ns(
        QEMU_CLOCK_VIRTUAL,
        rx_timer_cb,
        backend_ptr_raw as *mut c_void,
    );

    let backend = ZenohNetdevBackend {
        session,
        publisher,
        subscriber,
        node_id,
        nc,
        rx_timer,
        rx_queue: std::sync::Mutex::new(Vec::with_capacity(1024)),
    };

    ptr::write(backend_ptr_raw, backend);

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
        // Cancel any pending timer callback before freeing to prevent use-after-free
        // if the timer fires between now and the queue mutex being destroyed.
        virtmcu_timer_del(b.rx_timer);
        virtmcu_timer_free(b.rx_timer);
    }
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
    if queue.len() < 1024 {
        // Insertion sort by vtime (ascending)
        let pos = queue
            .binary_search_by(|probe| probe.delivery_vtime.cmp(&vtime))
            .unwrap_or_else(|e| e);
        queue.insert(
            pos,
            RxFrame {
                delivery_vtime: vtime,
                data: frame_data,
            },
        );

        // Mod timer for the earliest frame.
        // We are holding BQL here because _bql_guard is active.
        unsafe {
            virtmcu_timer_mod(backend.rx_timer, queue[0].delivery_vtime as i64);
        }
    } else {
        // Queue overflow: drop oldest
        eprintln!(
            "[zenoh-netdev] RX queue overflow on node {}, dropping oldest frame",
            backend.node_id
        );
        queue.remove(0);
        let pos = queue
            .binary_search_by(|probe| probe.delivery_vtime.cmp(&vtime))
            .unwrap_or_else(|e| e);
        queue.insert(
            pos,
            RxFrame {
                delivery_vtime: vtime,
                data: frame_data,
            },
        );

        unsafe {
            virtmcu_timer_mod(backend.rx_timer, queue[0].delivery_vtime as i64);
        }
    }
}

extern "C" fn rx_timer_cb(opaque: *mut c_void) {
    let backend = unsafe { &*(opaque as *mut ZenohNetdevBackend) };

    loop {
        let frame = {
            let mut queue = backend.rx_queue.lock().unwrap();
            let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;

            if queue.is_empty() {
                return;
            }

            if queue[0].delivery_vtime <= now {
                queue.remove(0)
            } else {
                // Timer fired early or virtual time moved backwards (unlikely but handle)
                unsafe {
                    virtmcu_timer_mod(backend.rx_timer, queue[0].delivery_vtime as i64);
                }
                return;
            }
        };

        // Frames delivered early indicate a timer logic bug: the timer should only
        // fire once virtual time has caught up to the frame's scheduled delivery time.
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
        // Loop back: next iteration will either deliver the next due frame or re-arm.
    }
}
