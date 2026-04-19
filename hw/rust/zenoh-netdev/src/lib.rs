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
use std::ffi::{CStr, CString};
use std::ptr;
use virtmcu_qom::error::Error;
use virtmcu_qom::net::{
    qemu_new_net_client, virtmcu_zenoh_netdev_hook, NetClientInfo, NetClientState, Netdev,
    NET_CLIENT_DRIVER_ZENOH,
};
use virtmcu_qom::qdev::{DeviceClass, SysBusDevice};
use virtmcu_qom::qom::{Object, ObjectClass, TypeInfo};
use virtmcu_qom::{declare_device_type, device_class, error_setg};
use zenoh::pubsub::Subscriber;
use zenoh::Session;
use zenoh::Wait;

use crossbeam_channel::{unbounded, Receiver, Sender};
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
pub struct ZenohNetdevQEMU {
    pub parent_obj: SysBusDevice,
    pub nc: NetClientState,
    pub rust_state: *mut ZenohNetdevState,
}

#[repr(C)]
pub struct ZenohNetClient {
    pub nc: NetClientState,
    pub rust_state: *mut ZenohNetdevState,
}

struct OrderedPacket {
    vtime: u64,
    data: Vec<u8>,
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

pub struct ZenohNetdevState {
    session: Session,
    nc: *mut NetClientState,
    node_id: u32,
    topic: String,
    subscriber: Option<Subscriber<()>>,
    rx_timer: *mut QemuTimer,
    rx_receiver: Receiver<OrderedPacket>,
    local_heap: Mutex<BinaryHeap<OrderedPacket>>,
    earliest_vtime: Arc<AtomicU64>,
}

unsafe extern "C" fn zenoh_netdev_receive(
    nc: *mut NetClientState,
    buf: *const u8,
    size: usize,
) -> isize {
    // Find ZenohNetdevQEMU from nc using offset_of
    let s = &mut *(nc as *mut ZenohNetClient);
    if s.rust_state.is_null() {
        return 0;
    }
    zenoh_netdev_receive_internal(&*s.rust_state, buf, size)
}

unsafe extern "C" fn zenoh_netdev_can_receive(_nc: *mut NetClientState) -> bool {
    true
}

unsafe extern "C" fn zenoh_netdev_cleanup(nc: *mut NetClientState) {
    let s = &mut *(nc as *mut ZenohNetClient);
    if !s.rust_state.is_null() {
        let state = Box::from_raw(s.rust_state);
        if !state.rx_timer.is_null() {
            unsafe {
                virtmcu_timer_del(state.rx_timer);
                virtmcu_timer_free(state.rx_timer);
            }
        }
        s.rust_state = ptr::null_mut();
    }
}

static NET_ZENOH_INFO: NetClientInfo = NetClientInfo {
    type_id: NET_CLIENT_DRIVER_ZENOH,
    size: std::mem::size_of::<ZenohNetClient>(),
    receive: Some(zenoh_netdev_receive),
    receive_raw: ptr::null_mut(),
    receive_iov: ptr::null_mut(),
    cleanup: Some(zenoh_netdev_cleanup),
    can_receive: Some(zenoh_netdev_can_receive),
    _opaque: [0; 208 - 56],
};

unsafe extern "C" fn zenoh_netdev_hook(
    netdev: *const Netdev,
    name: *const c_char,
    peer: *mut NetClientState,
    errp: *mut *mut Error,
) -> c_int {
    let opts = &(*netdev).u.zenoh;

    let nc = qemu_new_net_client(&NET_ZENOH_INFO, peer, c"zenoh".as_ptr(), name);
    let s = &mut *(nc as *mut ZenohNetClient);

    let node_id = if opts.node.is_null() {
        0
    } else {
        CStr::from_ptr(opts.node)
            .to_string_lossy()
            .parse::<u32>()
            .unwrap_or(0)
    };

    let router = if opts.router.is_null() {
        ptr::null()
    } else {
        opts.router as *const c_char
    };

    let topic = if opts.topic.is_null() {
        "sim/eth/frame".to_string()
    } else {
        CStr::from_ptr(opts.topic).to_string_lossy().into_owned()
    };

    s.rust_state = zenoh_netdev_init_internal(nc, node_id, router, topic);
    if s.rust_state.is_null() {
        error_setg!(
            errp,
            c"zenoh-netdev: failed to initialize Rust backend".as_ptr()
        );
        return -1;
    }

    0
}

unsafe extern "C" fn zenoh_netdev_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let dc = device_class!(klass);
    unsafe {
        (*dc).user_creatable = true;
        virtmcu_zenoh_netdev_hook = Some(zenoh_netdev_hook);
    }
}

static ZENOH_NETDEV_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"zenoh-netdev".as_ptr(),
    parent: c"sys-bus-device".as_ptr(),
    instance_size: std::mem::size_of::<ZenohNetdevQEMU>(),
    instance_align: 0,
    instance_init: None,
    instance_post_init: None,
    instance_finalize: None,
    abstract_: false,
    class_size: 0,
    class_init: Some(zenoh_netdev_class_init),
    class_base_init: None,
    class_data: ptr::null(),
    interfaces: ptr::null(),
};

declare_device_type!(zenoh_netdev_type_init, ZENOH_NETDEV_TYPE_INFO);

/* ── Internal Logic ───────────────────────────────────────────────────────── */

extern "C" fn rx_timer_cb(opaque: *mut core::ffi::c_void) {
    let state = unsafe { &*(opaque as *mut ZenohNetdevState) };
    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;

    let mut heap = state.local_heap.lock().unwrap();

    // Drain MPSC channel into the priority queue (lock-free for Zenoh workers)
    while let Ok(packet) = state.rx_receiver.try_recv() {
        heap.push(packet);
    }

    while let Some(packet) = heap.peek() {
        if packet.vtime <= now {
            let packet = heap.pop().unwrap();
            unsafe {
                virtmcu_qom::net::qemu_send_packet(
                    state.nc,
                    packet.data.as_ptr(),
                    packet.data.len(),
                );
            }
        } else {
            break;
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

fn zenoh_netdev_init_internal(
    nc: *mut NetClientState,
    node_id: u32,
    router: *const c_char,
    topic: String,
) -> *mut ZenohNetdevState {
    let session = unsafe {
        match virtmcu_zenoh::open_session(router) {
            Ok(s) => s,
            Err(_) => return ptr::null_mut(),
        }
    };

    let (tx, rx) = unbounded();
    let local_heap = Mutex::new(BinaryHeap::new());
    let earliest_vtime = Arc::new(AtomicU64::new(u64::MAX));
    let earliest_clone = earliest_vtime.clone();

    let timer_ptr_clone = Arc::new(AtomicUsize::new(0));
    let timer_ptr = timer_ptr_clone.clone();

    let subscriber = session
        .declare_subscriber(&topic)
        .callback(move |sample| {
            let tp = timer_ptr_clone.load(AtomicOrdering::Acquire);
            if tp == 0 {
                return;
            }
            let rx_timer = tp as *mut QemuTimer;

            let data = sample.payload().to_bytes();
            if data.len() < 12 {
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

    let mut state = Box::new(ZenohNetdevState {
        session,
        nc,
        node_id,
        topic: topic.clone(),
        subscriber,
        rx_timer: ptr::null_mut(),
        rx_receiver: rx,
        local_heap,
        earliest_vtime,
    });

    let state_ptr = &mut *state as *mut ZenohNetdevState;
    let rx_timer =
        unsafe { virtmcu_timer_new_ns(QEMU_CLOCK_VIRTUAL, rx_timer_cb, state_ptr as *mut c_void) };

    state.rx_timer = rx_timer;
    timer_ptr.store(rx_timer as usize, AtomicOrdering::Release);

    Box::into_raw(state)
}

fn zenoh_netdev_receive_internal(state: &ZenohNetdevState, buf: *const u8, size: usize) -> isize {
    let tx_topic = format!("{}/{}/tx", state.topic, state.node_id);
    let payload = unsafe { std::slice::from_raw_parts(buf, size) };

    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;

    let header = ZenohFrameHeader {
        delivery_vtime_ns: now,
        size: size as u32,
    };

    let mut data = Vec::with_capacity(12 + size);
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
    size as isize
}
