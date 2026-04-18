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

use virtmcu_qom::chardev::*;
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

pub struct ZenohChardevState {
    chr: *mut Chardev,
    #[allow(dead_code)]
    session: Session,
    // Safety: Publisher<'static> is a lie — the publisher holds an Arc back to the session.
    // Both live in this heap-allocated struct. Rust drops fields top-to-bottom, so session
    // outlives publisher within the same struct, which is the only invariant that matters.
    publisher: Publisher<'static>,
    #[allow(dead_code)]
    subscriber: Subscriber<()>,
    rx_timer: *mut QemuTimer,
    #[allow(dead_code)]
    node_id: String,

    #[allow(dead_code)]
    mutex: *mut QemuMutex,
    rx_queue: std::sync::Mutex<Vec<RxFrame>>,
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_chardev_init(
    chr: *mut Chardev,
    node_id: *const c_char,
    router: *const c_char,
    topic: *const c_char,
) -> *mut ZenohChardevState {
    let node_id_str = CStr::from_ptr(node_id).to_str().unwrap().to_owned();

    let session = match virtmcu_zenoh::open_session(router) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "[zenoh-chardev] node={}: FAILED to open Zenoh session: {}",
                node_id_str, e
            );
            return ptr::null_mut();
        }
    };

    let topic_tx;
    let topic_rx;
    if !topic.is_null() {
        let t = CStr::from_ptr(topic).to_str().unwrap();
        topic_tx = format!("{}/tx", t);
        topic_rx = format!("{}/rx", t);
    } else {
        topic_tx = format!("virtmcu/uart/{}/tx", node_id_str);
        topic_rx = format!("virtmcu/uart/{}/rx", node_id_str);
    }

    let publisher = session.declare_publisher(topic_tx).wait().unwrap();

    let mutex = virtmcu_mutex_new();

    // Two-phase init: allocate first to get a stable address the subscriber callback can
    // capture, then write the fully-constructed state into it. Box::new_uninit() panics on
    // OOM (via handle_alloc_error) instead of returning null, eliminating null-deref UB.
    let state_ptr_raw: *mut ZenohChardevState =
        Box::into_raw(Box::<std::mem::MaybeUninit<ZenohChardevState>>::new_uninit()).cast();
    let state_ptr_usize = state_ptr_raw as usize;

    let subscriber = session
        .declare_subscriber(topic_rx)
        .callback(move |sample| {
            let state = &*(state_ptr_usize as *const ZenohChardevState);
            on_rx_frame(state, sample);
        })
        .wait()
        .unwrap();

    let rx_timer = virtmcu_timer_new_ns(
        QEMU_CLOCK_VIRTUAL,
        rx_timer_cb,
        state_ptr_raw as *mut c_void,
    );

    let state = ZenohChardevState {
        chr,
        session,
        publisher,
        subscriber,
        rx_timer,
        node_id: node_id_str,
        mutex,
        rx_queue: std::sync::Mutex::new(Vec::with_capacity(1024)),
    };

    ptr::write(state_ptr_raw, state);

    state_ptr_raw
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_chardev_write_rust(
    state: *mut ZenohChardevState,
    buf: *const u8,
    len: i32,
) -> i32 {
    assert!(!state.is_null(), "state pointer is null");
    assert!(!buf.is_null(), "tx buffer is null");
    assert!(
        (0..=1024 * 1024).contains(&len),
        "tx buffer size out of bounds"
    );

    let s = &*state;
    let vtime = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL);

    let size = len as usize;
    let mut msg = Vec::with_capacity(12 + size);

    let mut hdr_bytes = [0u8; 12];
    LittleEndian::write_u64(&mut hdr_bytes[0..8], vtime as u64);
    LittleEndian::write_u32(&mut hdr_bytes[8..12], size as u32);

    msg.extend_from_slice(&hdr_bytes);
    msg.extend_from_slice(std::slice::from_raw_parts(buf, size));

    let _ = s.publisher.put(msg).wait();

    len
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_chardev_cleanup_rust(state: *mut ZenohChardevState) {
    if state.is_null() {
        return;
    }
    let s = Box::from_raw(state);
    if !s.rx_timer.is_null() {
        virtmcu_timer_free(s.rx_timer);
    }
    virtmcu_mutex_free(s.mutex);
}

fn on_rx_frame(state: &ZenohChardevState, sample: zenoh::sample::Sample) {
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

    let mut queue = state.rx_queue.lock().unwrap();
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

        // Mod timer for the earliest frame
        unsafe {
            virtmcu_timer_mod(state.rx_timer, queue[0].delivery_vtime as i64);
        }
    }
}

extern "C" fn rx_timer_cb(opaque: *mut c_void) {
    let state = unsafe { &*(opaque as *mut ZenohChardevState) };

    loop {
        let frame = {
            let mut queue = state.rx_queue.lock().unwrap();
            let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;

            if queue.is_empty() {
                return;
            }

            if queue[0].delivery_vtime <= now {
                queue.remove(0)
            } else {
                unsafe {
                    virtmcu_timer_mod(state.rx_timer, queue[0].delivery_vtime as i64);
                }
                return;
            }
        };

        unsafe {
            qemu_chr_be_write(state.chr, frame.data.as_ptr(), frame.data.len());
        }
    }
}

#[cfg(test)]
mod tests {
    #![deny(unsafe_code)]
    use byteorder::{ByteOrder, LittleEndian};

    #[test]
    fn frame_header_encode_decode() {
        let vtime: u64 = 1_234_567_890_000;
        let size: u32 = 64;
        let mut hdr = [0u8; 12];
        LittleEndian::write_u64(&mut hdr[0..8], vtime);
        LittleEndian::write_u32(&mut hdr[8..12], size);
        assert_eq!(LittleEndian::read_u64(&hdr[0..8]), vtime);
        assert_eq!(LittleEndian::read_u32(&hdr[8..12]), size);
    }

    #[test]
    fn rx_queue_priority_order() {
        // Verify frames inserted out of vtime order sort correctly.
        let mut queue: Vec<(u64, u8)> = Vec::new();
        let frames = [(200u64, b'A'), (100u64, b'B'), (150u64, b'C')];
        for (vt, id) in frames {
            let pos = queue
                .binary_search_by(|p| p.0.cmp(&vt))
                .unwrap_or_else(|e| e);
            queue.insert(pos, (vt, id));
        }
        assert_eq!(queue[0].1, b'B'); // earliest vtime first
        assert_eq!(queue[1].1, b'C');
        assert_eq!(queue[2].1, b'A');
    }
}
