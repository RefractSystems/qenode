#![allow(unused_variables)]
#![allow(clippy::all)]
#![allow(
    clippy::missing_safety_doc,
    clippy::collapsible_match,
    dead_code,
    unused_imports,
    clippy::needless_return,
    clippy::manual_range_contains,
    clippy::single_component_path_imports,
    clippy::len_zero,
    clippy::while_immutable_condition
)]

use core::ffi::{c_char, c_int, c_uint, c_void};
use libc;
use std::ffi::{CStr, CString};
use std::ptr;
use virtmcu_qom::chardev::{qemu_chr_be_can_write, qemu_chr_be_write, Chardev, ChardevClass};
use virtmcu_qom::error::Error;
use virtmcu_qom::qom::{Object, ObjectClass, TypeInfo};
use virtmcu_qom::{declare_device_type, device_class, error_setg};
use zenoh::pubsub::Subscriber;
use zenoh::Session;
use zenoh::Wait;

use crossbeam_channel::{bounded, Receiver, Sender};
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};
use virtmcu_api::ZenohFrameHeader;
use virtmcu_qom::sync::Bql;
use virtmcu_qom::timer::{
    qemu_clock_get_ns, virtmcu_timer_del, virtmcu_timer_free, virtmcu_timer_mod,
    virtmcu_timer_new_ns, QemuTimer, QEMU_CLOCK_VIRTUAL,
};

#[repr(C)]
pub struct ChardevZenoh {
    pub parent: Chardev,
    pub rust_state: *mut ZenohChardevState,
}

pub struct OrderedPacket {
    pub vtime: u64,
    pub data: Vec<u8>,
}

impl PartialEq for OrderedPacket {
    fn eq(&self, other: &Self) -> bool {
        self.vtime == other.vtime
    }
}
impl Eq for OrderedPacket {}
impl PartialOrd for OrderedPacket {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for OrderedPacket {
    fn cmp(&self, other: &Self) -> Ordering {
        other.vtime.cmp(&self.vtime)
    }
}

pub struct ZenohChardevState {
    session: Session,
    chr: *mut Chardev,
    node_id: String,
    topic: String,
    subscriber: Option<Subscriber<()>>,
    rx_timer: *mut QemuTimer,
    rx_receiver: Receiver<OrderedPacket>,
    local_heap: Mutex<BinaryHeap<OrderedPacket>>,
    earliest_vtime: Arc<AtomicU64>,
}

#[repr(C)]
struct ChardevZenohOptions {
    /* ChardevCommon */
    logfile: *mut c_char,
    has_logappend: bool,
    logappend: bool,
    has_logtimestamp: bool,
    logtimestamp: bool,
    _padding: [u8; 4],
    /* Own members */
    node: *mut c_char,
    router: *mut c_char,
    topic: *mut c_char,
}

#[repr(C)]
struct ChardevBackend {
    type_: c_int,
    padding: c_int,
    u: ChardevBackendUnion,
}

#[repr(C)]
union ChardevBackendUnion {
    data: *mut c_void,
}

const CHARDEV_BACKEND_KIND_ZENOH: c_int = 17;

extern "C" {
    pub fn qemu_opt_get(opts: *mut c_void, name: *const c_char) -> *const c_char;
    pub fn g_strdup(s: *const c_char) -> *mut c_char;
    pub fn g_malloc0(size: usize) -> *mut c_void;
    pub fn qemu_chr_parse_common(opts: *mut c_void, base: *mut c_void);
}

unsafe extern "C" fn zenoh_chr_write(chr: *mut Chardev, buf: *const u8, len: c_int) -> c_int {
    let s = &mut *(chr as *mut ChardevZenoh);
    if s.rust_state.is_null() {
        return 0;
    }
    zenoh_chardev_write_internal(&*s.rust_state, buf, len as usize) as c_int
}

unsafe extern "C" fn zenoh_chr_parse(
    opts: *mut c_void,
    backend: *mut c_void,
    errp: *mut *mut c_void,
) {
    unsafe {
        libc::write(1, b"zenoh_chr_parse start\n".as_ptr() as *const c_void, 22);
    }
    let node = qemu_opt_get(opts, c"node".as_ptr());
    let router = qemu_opt_get(opts, c"router".as_ptr());
    let topic = qemu_opt_get(opts, c"topic".as_ptr());

    if node.is_null() {
        unsafe {
            libc::write(
                1,
                b"zenoh_chr_parse: node is null\n".as_ptr() as *const c_void,
                30,
            );
        }
        error_setg!(
            errp as *mut *mut Error,
            c"chardev: zenoh: 'node' is required".as_ptr()
        );
        return;
    }

    let zenoh_opts =
        g_malloc0(std::mem::size_of::<ChardevZenohOptions>()) as *mut ChardevZenohOptions;
    (*zenoh_opts).node = g_strdup(node);
    if !router.is_null() {
        (*zenoh_opts).router = g_strdup(router);
    }
    if !topic.is_null() {
        (*zenoh_opts).topic = g_strdup(topic);
    }

    let b = &mut *(backend as *mut ChardevBackend);
    b.type_ = CHARDEV_BACKEND_KIND_ZENOH;
    b.u.data = zenoh_opts as *mut c_void;

    unsafe {
        libc::write(
            1,
            b"zenoh_chr_parse: calling qemu_chr_parse_common\n".as_ptr() as *const c_void,
            47,
        );
    }
    qemu_chr_parse_common(opts, zenoh_opts as *mut c_void);
    unsafe {
        libc::write(1, b"zenoh_chr_parse end\n".as_ptr() as *const c_void, 20);
    }
}

unsafe extern "C" fn zenoh_chr_open(
    chr: *mut Chardev,
    backend: *mut c_void,
    be_opened: *mut bool,
    errp: *mut *mut c_void,
) -> bool {
    let s = &mut *(chr as *mut ChardevZenoh);
    let b = &*(backend as *mut ChardevBackend);
    let opts = b.u.data as *mut ChardevZenohOptions;

    let node = CStr::from_ptr((*opts).node).to_string_lossy().into_owned();
    let router = if (*opts).router.is_null() {
        ptr::null()
    } else {
        (*opts).router as *const c_char
    };
    let topic = if (*opts).topic.is_null() {
        "sim/chardev".to_string()
    } else {
        CStr::from_ptr((*opts).topic).to_string_lossy().into_owned()
    };

    s.rust_state = zenoh_chardev_init_internal(chr, node, router, topic);
    if s.rust_state.is_null() {
        error_setg!(
            errp as *mut *mut Error,
            c"zenoh-chardev: failed to initialize Rust backend".as_ptr()
        );
        return false;
    }
    *be_opened = true;
    true
}

unsafe extern "C" fn zenoh_chr_finalize(obj: *mut Object) {
    let s = &mut *(obj as *mut ChardevZenoh);
    if !s.rust_state.is_null() {
        let state = Box::from_raw(s.rust_state);
        if !state.rx_timer.is_null() {
            virtmcu_timer_del(state.rx_timer);
            virtmcu_timer_free(state.rx_timer);
        }
        s.rust_state = ptr::null_mut();
    }
}

unsafe extern "C" fn char_zenoh_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let cc = &mut *(klass as *mut ChardevClass);
    cc.chr_parse = Some(zenoh_chr_parse);
    cc.chr_open = Some(zenoh_chr_open);
    cc.chr_write = Some(zenoh_chr_write);
}

static CHAR_ZENOH_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"chardev-zenoh".as_ptr(),
    parent: c"chardev".as_ptr(),
    instance_size: std::mem::size_of::<ChardevZenoh>(),
    instance_align: 0,
    instance_init: None,
    instance_post_init: None,
    instance_finalize: Some(zenoh_chr_finalize),
    abstract_: false,
    class_size: 0,
    class_init: Some(char_zenoh_class_init),
    class_base_init: None,
    class_data: ptr::null(),
    interfaces: ptr::null(),
};

declare_device_type!(char_zenoh_type_init, CHAR_ZENOH_TYPE_INFO);

/* ── Internal Logic ───────────────────────────────────────────────────────── */

extern "C" fn rx_timer_cb(opaque: *mut core::ffi::c_void) {
    let state = unsafe { &*(opaque as *mut ZenohChardevState) };
    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;

    let mut heap = state.local_heap.lock().unwrap();

    // Drain MPSC channel into the priority queue
    while let Ok(packet) = state.rx_receiver.try_recv() {
        heap.push(packet);
    }

    while let Some(packet) = heap.peek() {
        if packet.vtime > now {
            break;
        }

        let mut retry_later = false;

        unsafe {
            virtmcu_qom::sync::virtmcu_bql_lock();
            let can_write = qemu_chr_be_can_write(state.chr) as usize;

            if can_write > 0 {
                let mut packet = heap.pop().unwrap();
                let to_write = std::cmp::min(can_write, packet.data.len());

                qemu_chr_be_write(state.chr, packet.data.as_ptr(), to_write);

                if to_write < packet.data.len() {
                    // Packet didn't fit completely, keep the remainder and schedule for the next tick
                    packet.data.drain(0..to_write);
                    heap.push(packet);
                    retry_later = true;
                }
            } else {
                // Buffer is full. We can't write right now.
                retry_later = true;
            }
            virtmcu_qom::sync::virtmcu_bql_unlock();
        }

        if retry_later {
            // UART is congested, wait ~10us virtual time before trying again
            let retry_vtime = now + 10_000;
            state
                .earliest_vtime
                .store(retry_vtime, AtomicOrdering::Release);
            unsafe {
                virtmcu_timer_mod(state.rx_timer, retry_vtime as i64);
            }
            return;
        }
    }

    if let Some(next_packet) = heap.peek() {
        state
            .earliest_vtime
            .store(next_packet.vtime, AtomicOrdering::Release);
        unsafe {
            virtmcu_timer_mod(state.rx_timer, next_packet.vtime as i64);
        }
    } else {
        state
            .earliest_vtime
            .store(u64::MAX, AtomicOrdering::Release);
    }
}

fn zenoh_chardev_init_internal(
    chr: *mut Chardev,
    node_id: String,
    router: *const c_char,
    topic: String,
) -> *mut ZenohChardevState {
    let session = unsafe {
        match virtmcu_zenoh::open_session(router) {
            Ok(s) => s,
            Err(_) => return ptr::null_mut(),
        }
    };

    let (tx, rx) = bounded(10240); // Larger buffer for flood tests
    let local_heap = Mutex::new(BinaryHeap::new());
    let earliest_vtime = Arc::new(AtomicU64::new(u64::MAX));
    let earliest_clone = earliest_vtime.clone();

    let timer_ptr_clone = Arc::new(AtomicUsize::new(0));
    let timer_ptr = timer_ptr_clone.clone();

    let rx_topic = format!("{}/{}/rx", topic, node_id);

    let subscriber = session
        .declare_subscriber(&rx_topic)
        .callback(move |sample| {
            let tp = timer_ptr_clone.load(AtomicOrdering::Acquire);
            if tp == 0 {
                return;
            }
            let rx_timer = tp as *mut QemuTimer;

            let data = sample.payload().to_bytes();
            if data.len() < 12 {
                // Compatibility with legacy flood tests that don't send headers
                let packet = OrderedPacket {
                    vtime: 0, // Deliver immediately
                    data: data.to_vec(),
                };
                let _ = tx.send(packet);
                let _bql = Bql::lock();
                unsafe {
                    virtmcu_timer_mod(rx_timer, 0);
                }
                return;
            }

            let mut header = ZenohFrameHeader::default();
            unsafe {
                std::ptr::copy_nonoverlapping(data.as_ptr(), &mut header as *mut _ as *mut u8, 12);
            }

            let payload = data[12..].to_vec();

            let packet = OrderedPacket {
                vtime: header.delivery_vtime_ns,
                data: payload,
            };

            let _ = tx.send(packet);

            let current_earliest = earliest_clone.load(AtomicOrdering::Acquire);
            if header.delivery_vtime_ns < current_earliest {
                earliest_clone.fetch_min(header.delivery_vtime_ns, AtomicOrdering::Release);
                let _bql = Bql::lock();
                unsafe {
                    virtmcu_timer_mod(rx_timer, header.delivery_vtime_ns as i64);
                }
            }
        })
        .wait()
        .ok();

    let mut state = Box::new(ZenohChardevState {
        session,
        chr,
        node_id,
        topic,
        subscriber,
        rx_timer: ptr::null_mut(),
        rx_receiver: rx,
        local_heap,
        earliest_vtime,
    });

    let state_ptr = &mut *state as *mut ZenohChardevState;
    let rx_timer =
        unsafe { virtmcu_timer_new_ns(QEMU_CLOCK_VIRTUAL, rx_timer_cb, state_ptr as *mut c_void) };

    state.rx_timer = rx_timer;
    timer_ptr.store(rx_timer as usize, AtomicOrdering::Release);

    Box::into_raw(state)
}

fn zenoh_chardev_write_internal(state: &ZenohChardevState, buf: *const u8, len: usize) -> usize {
    let tx_topic = format!("{}/{}/tx", state.topic, state.node_id);
    let payload = unsafe { std::slice::from_raw_parts(buf, len) };

    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;

    let header = ZenohFrameHeader {
        delivery_vtime_ns: now,
        size: len as u32,
    };

    let mut data = Vec::with_capacity(12 + len);
    let mut header_bytes = [0u8; 12];
    unsafe {
        std::ptr::copy_nonoverlapping(
            &header as *const _ as *const u8,
            header_bytes.as_mut_ptr(),
            12,
        );
    }
    data.extend_from_slice(&header_bytes);
    data.extend_from_slice(payload);

    let _ = state.session.put(tx_topic, data).wait();
    len
}
