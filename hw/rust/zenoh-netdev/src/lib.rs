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
use virtmcu_zenoh::SafeSubscriber;
use zenoh::Session;
use zenoh::Wait;

use crossbeam_channel::{bounded, Receiver, Sender};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, VecDeque};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering as AtomicOrdering};
use std::sync::Arc;
use virtmcu_api::ZenohFrameHeader;
use virtmcu_qom::sync::BqlGuarded;
use virtmcu_qom::timer::{qemu_clock_get_ns, QomTimer, QEMU_CLOCK_VIRTUAL};

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

pub struct ZenohNetdevState {
    session: Session,
    nc: *mut NetClientState,
    node_id: u32,
    topic: String,
    subscriber: Option<SafeSubscriber>,
    rx_timer: Option<Arc<QomTimer>>,
    rx_receiver: Receiver<OrderedPacket>,
    // All state accessed exclusively under BQL; see BqlGuarded docs.
    local_heap: BqlGuarded<BinaryHeap<OrderedPacket>>,
    backlog: BqlGuarded<VecDeque<Vec<u8>>>,
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

unsafe extern "C" fn zenoh_netdev_can_receive(nc: *mut NetClientState) -> bool {
    let s = &mut *(nc as *mut ZenohNetClient);
    if s.rust_state.is_null() {
        return true;
    }
    let backlog = (*s.rust_state).backlog.get();
    backlog.is_empty()
}

unsafe extern "C" fn zenoh_netdev_cleanup(nc: *mut NetClientState) {
    let s = &mut *(nc as *mut ZenohNetClient);
    if !s.rust_state.is_null() {
        let mut state = Box::from_raw(s.rust_state);
        // Explicitly drop subscriber first to wait for callbacks
        state.subscriber.take();
        state.rx_timer.take();
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

    let nc = qemu_new_net_client(&raw const NET_ZENOH_INFO, peer, c"zenoh".as_ptr(), name);
    let s = &mut *(nc as *mut ZenohNetClient);

    let node_id = if opts.node.is_null() {
        0
    } else {
        CStr::from_ptr(opts.node).to_string_lossy().parse::<u32>().unwrap_or(0)
    };

    let router = if opts.router.is_null() { ptr::null() } else { opts.router.cast_const() };

    let topic = if opts.topic.is_null() {
        "sim/eth/frame".to_string()
    } else {
        CStr::from_ptr(opts.topic).to_string_lossy().into_owned()
    };

    s.rust_state = zenoh_netdev_init_internal(nc, node_id, router, topic);
    if s.rust_state.is_null() {
        error_setg!(errp, "zenoh-netdev: failed to initialize Rust backend");
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

fn drain_net_backlog(state: &ZenohNetdevState) -> bool {
    let mut backlog = state.backlog.get_mut();
    while let Some(packet) = backlog.front() {
        if unsafe { !virtmcu_qom::net::qemu_can_receive_packet(state.nc) } {
            return false;
        }
        let data = backlog.pop_front().unwrap_or_else(|| std::process::abort());
        unsafe {
            virtmcu_qom::net::qemu_send_packet(state.nc, data.as_ptr(), data.len());
        }
    }
    true
}

extern "C" fn rx_timer_cb(opaque: *mut core::ffi::c_void) {
    debug_assert!(virtmcu_qom::sync::Bql::is_held(), "BQL must be held during timer callbacks");
    let state = unsafe { &*(opaque as *mut ZenohNetdevState) };
    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;

    // 1. Drain backlog
    if !drain_net_backlog(state) {
        if let Some(rx_timer) = &state.rx_timer {
            rx_timer.mod_ns(now as i64 + 1_000_000); // 1ms
        }
        return;
    }

    let mut heap = state.local_heap.get_mut();

    // Drain MPSC channel into the priority queue (lock-free for Zenoh workers)
    while let Ok(packet) = state.rx_receiver.try_recv() {
        heap.push(packet);
    }

    while let Some(packet) = heap.peek() {
        if packet.vtime <= now {
            if unsafe { !virtmcu_qom::net::qemu_can_receive_packet(state.nc) } {
                let mut backlog = state.backlog.get_mut();
                let p = heap.pop().unwrap_or_else(|| std::process::abort());
                backlog.push_back(p.data);
                break;
            }

            let p = heap.pop().unwrap_or_else(|| std::process::abort());
            unsafe {
                virtmcu_qom::net::qemu_send_packet(state.nc, p.data.as_ptr(), p.data.len());
            }
        } else {
            // Re-arm timer
            if let Some(rx_timer) = &state.rx_timer {
                rx_timer.mod_ns(packet.vtime as i64);
            }
            break;
        }
    }

    if heap.is_empty() {
        state.earliest_vtime.store(u64::MAX, AtomicOrdering::Release);
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

    let (tx, rx) = bounded(65536);
    let local_heap = BqlGuarded::new(BinaryHeap::new());
    let earliest_vtime = Arc::new(AtomicU64::new(u64::MAX));
    let earliest_clone = std::sync::Arc::clone(&earliest_vtime);

    let mut state = Box::new(ZenohNetdevState {
        session: session.clone(),
        nc,
        node_id,
        topic: topic.clone(),
        subscriber: None,
        rx_timer: None,
        rx_receiver: rx,
        local_heap,
        backlog: BqlGuarded::new(VecDeque::new()),
        earliest_vtime,
    });

    let state_ptr = &raw mut *state;
    let rx_timer = Arc::new(unsafe {
        QomTimer::new(QEMU_CLOCK_VIRTUAL, rx_timer_cb, state_ptr as *mut c_void)
    });
    let rx_timer_clone = Arc::clone(&rx_timer);

    let subscriber = SafeSubscriber::new(&session, &topic, move |sample| {
        let data = sample.payload().to_bytes();
        if data.len() < 12 {
            virtmcu_qom::vlog!(
                "[zenoh-netdev] Warning: Dropping malformed packet (too short: {} bytes)\n",
                data.len()
            );
            return;
        }

        let header = match ZenohFrameHeader::unpack_slice(&data[..12]) {
            Some(h) => h,
            None => return,
        };

        let payload = data[12..].to_vec();

        let packet = OrderedPacket { vtime: header.delivery_vtime_ns, data: payload };

        if tx.send(packet).is_ok() {
            let current_earliest = earliest_clone.load(AtomicOrdering::Acquire);
            if header.delivery_vtime_ns < current_earliest {
                earliest_clone.fetch_min(header.delivery_vtime_ns, AtomicOrdering::Release);
                rx_timer_clone.mod_ns(header.delivery_vtime_ns as i64);
            }
        } else {
            virtmcu_qom::vlog!("[zenoh-netdev] Warning: RX channel full, dropping packet\n");
        }
    })
    .ok();

    state.rx_timer = Some(rx_timer);
    state.subscriber = subscriber;

    Box::into_raw(state)
}

fn zenoh_netdev_receive_internal(state: &ZenohNetdevState, buf: *const u8, size: usize) -> isize {
    let tx_topic = format!("{}/{}/tx", state.topic, state.node_id);
    let payload = unsafe { std::slice::from_raw_parts(buf, size) };

    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;

    let header = ZenohFrameHeader { delivery_vtime_ns: now, size: size as u32 };

    let mut data = Vec::with_capacity(12 + size);
    data.extend_from_slice(&header.pack());
    data.extend_from_slice(payload);

    let _ = state.session.put(tx_topic, data).wait();
    size as isize
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BinaryHeap;

    #[test]
    fn test_ordered_packet_ord() {
        let mut heap = BinaryHeap::new();
        heap.push(OrderedPacket { vtime: 1000, data: vec![1] });
        heap.push(OrderedPacket { vtime: 500, data: vec![2] });
        heap.push(OrderedPacket { vtime: 2000, data: vec![3] });

        // Lowest vtime (500) should pop first (min-heap)
        assert_eq!(heap.pop().unwrap_or_else(|| std::process::abort()).vtime, 500);
        assert_eq!(heap.pop().unwrap_or_else(|| std::process::abort()).vtime, 1000);
        assert_eq!(heap.pop().unwrap_or_else(|| std::process::abort()).vtime, 2000);
    }

    #[test]
    fn test_zenoh_net_client_layout() {
        // QEMU passes `*mut NetClientState` to our callbacks, and we cast it
        // to `*mut ZenohNetClient`. The `nc` field MUST be at offset 0.
        assert_eq!(
            core::mem::offset_of!(ZenohNetClient, nc),
            0,
            "NetClientState must be the first field in ZenohNetClient"
        );
    }

    #[test]
    fn test_zenoh_netdev_qemu_layout() {
        // QOM layout validation
        assert_eq!(
            core::mem::offset_of!(ZenohNetdevQEMU, parent_obj),
            0,
            "SysBusDevice must be the first field"
        );
    }
}
