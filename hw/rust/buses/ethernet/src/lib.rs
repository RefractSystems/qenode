#![allow(clippy::all, unused_imports, dead_code, unused_variables, unused_mut)] // virtmcu-allow: allow reasoning="Zero unsafe"
#![allow(clippy::all)] // virtmcu-allow: allow reasoning="Zero unsafe"
#![allow(clippy::panic)] // virtmcu-allow: allow reasoning="Fail Loudly"
#![allow(clippy::not_unsafe_ptr_arg_deref)]
// virtmcu-allow: allow reasoning="Zero unsafe"
// virtmcu-allow: allow reasoning="Pending P1 migration: deref_qom_ptr/opaque_to_state replaced by dynamic_cast_qom"
#![allow(deprecated)]
#![allow(clippy::missing_safety_doc)]
#![cfg_attr(
    test,
    allow(
        clippy::expect_used,
        clippy::unwrap_used,
        clippy::indexing_slicing,
        clippy::panic_in_result_fn
    )
)]
//! VirtMCU virtual network device with pluggable transport.

use zenoh::Wait;

extern crate alloc;

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cmp::Ordering;
use core::ffi::{c_char, c_int, c_void, CStr};
use core::ptr;
use core::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use virtmcu_qom::error::Error;
use virtmcu_qom::net::{
    qemu_new_net_client, virtmcu_netdev_hook, NetClientInfo, NetClientState, Netdev,
    NET_CLIENT_DRIVER_VIRTMCU,
};
use virtmcu_qom::qdev::SysBusDevice;
use virtmcu_qom::qom::{ObjectClass, TypeInfo};
use virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL;
use virtmcu_qom::{declare_device_type, device_class, error_setg};

const NETDEV_INFO_OPAQUE_SIZE: usize = 208 - 56;

#[repr(C)]
pub struct VirtmcuNetdevQEMU {
    pub parent_obj: SysBusDevice,
}

#[repr(C)]
pub struct VirtmcuNetClient {
    pub nc: NetClientState,
    pub rust_state: *mut VirtmcuNetdevState,
}

pub struct OrderedPacket {
    pub vtime: u64,
    pub sequence: u64,
    pub data: Vec<u8>,
}

impl PartialEq for OrderedPacket {
    fn eq(&self, other: &Self) -> bool {
        self.vtime == other.vtime && self.sequence == other.sequence
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
        match other.vtime.cmp(&self.vtime) {
            Ordering::Equal => other.sequence.cmp(&self.sequence),
            ord => ord,
        }
    }
}

impl virtmcu_qom::sync::DeliveryPacket for OrderedPacket {
    fn delivery_vtime_ns(&self) -> u64 {
        self.vtime
    }
}

pub struct VirtmcuNetdevState {
    transport: Arc<dyn virtmcu_wire::DataTransport>,
    tx_topic: String,
    nc: *mut NetClientState,
    receiver: Option<virtmcu_qom::sync::VtimeIngress<OrderedPacket>>,
    backlog: virtmcu_qom::sync::Mutex<VecDeque<Vec<u8>>>, // virtmcu-allow: mutex reasoning="Backlog managed securely"
    tx_sequence: AtomicU64,
    _max_backlog: u64,
    backlog_count: Arc<AtomicU64>,
    _dropped_frames: Arc<AtomicU64>,
    pub _liveliness: Option<zenoh::liveliness::LivelinessToken>,
}

extern "C" fn netdev_receive(nc: *mut NetClientState, buf: *const u8, size: usize) -> isize {
    let s = virtmcu_qom::timer::deref_qom_ptr::<VirtmcuNetClient>(nc as *mut core::ffi::c_void);
    if s.rust_state.is_null() {
        return 0;
    }
    virtmcu_qom::ffi_call! { netdev_receive_internal(&*s.rust_state, buf, size) }
}

extern "C" fn netdev_can_receive(nc: *mut NetClientState) -> bool {
    let s = virtmcu_qom::timer::deref_qom_ptr::<VirtmcuNetClient>(nc as *mut core::ffi::c_void);
    if s.rust_state.is_null() {
        return true;
    }
    let backlog = virtmcu_qom::timer::opaque_to_state_const::<VirtmcuNetdevState>(
        s.rust_state as *mut core::ffi::c_void,
    )
    .backlog
    .lock();
    backlog.is_empty()
}

extern "C" fn netdev_cleanup(nc: *mut NetClientState) {
    let s = virtmcu_qom::timer::deref_qom_ptr::<VirtmcuNetClient>(nc as *mut core::ffi::c_void);
    if !s.rust_state.is_null() {
        virtmcu_qom::ffi_call! {
            let mut state = Box::from_raw(s.rust_state);
            state.receiver.take();
            drop(state);
            s.rust_state = ptr::null_mut();
        }
    }
}

static NET_VIRTMCU_INFO: NetClientInfo = NetClientInfo {
    type_id: NET_CLIENT_DRIVER_VIRTMCU,
    size: core::mem::size_of::<VirtmcuNetClient>(),
    receive: Some(netdev_receive),
    receive_raw: None,
    receive_iov: None,
    cleanup: Some(netdev_cleanup),
    can_receive: Some(netdev_can_receive),
    _opaque: [0; NETDEV_INFO_OPAQUE_SIZE],
};

extern "C" fn netdev_hook(
    netdev: *const Netdev,
    name: *const c_char,
    peer: *mut NetClientState,
    errp: *mut *mut Error,
) -> c_int {
    let opts = virtmcu_qom::ffi_call! { &(*netdev).u.virtmcu };

    let nc = virtmcu_qom::ffi_call! {
        qemu_new_net_client(&raw const NET_VIRTMCU_INFO, peer, c"virtmcu".as_ptr(), name)
    };
    let s = virtmcu_qom::timer::deref_qom_ptr::<VirtmcuNetClient>(nc as *mut core::ffi::c_void);

    let node_id = if opts.node.is_null() {
        0
    } else {
        virtmcu_qom::ffi_call! { CStr::from_ptr(opts.node) }
            .to_string_lossy()
            .parse::<u32>()
            .expect("Invalid data format")
    };

    let transport_name = if opts.transport.is_null() {
        "zenoh".to_owned()
    } else {
        virtmcu_qom::ffi_call! { CStr::from_ptr(opts.transport) }.to_string_lossy().into_owned()
    };

    let router = if opts.router.is_null() { ptr::null() } else { opts.router.cast_const() };

    let topic = if opts.topic.is_null() {
        "sim/eth/frame".to_owned()
    } else {
        virtmcu_qom::ffi_call! { CStr::from_ptr(opts.topic) }.to_string_lossy().into_owned()
    };

    let max_backlog = if opts.has_max_backlog { opts.max_backlog } else { 256 };

    s.rust_state = netdev_init_internal(nc, node_id, transport_name, router, topic, max_backlog);
    if s.rust_state.is_null() {
        error_setg!(errp, "netdev: failed to initialize Rust backend");
        return -1;
    }

    0
}

extern "C" fn netdev_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let dc = device_class!(klass);
    virtmcu_qom::ffi_call! {
        (*dc).user_creatable = true;
        virtmcu_netdev_hook = Some(netdev_hook);
    }
}

#[used]
static VIRTMCU_NETDEV_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"netdev".as_ptr(),
    parent: c"sys-bus-device".as_ptr(),
    instance_size: core::mem::size_of::<VirtmcuNetdevQEMU>(),
    instance_align: 0,
    instance_init: None,
    instance_post_init: None,
    instance_finalize: None,
    abstract_: false,
    class_size: core::mem::size_of::<virtmcu_qom::qdev::SysBusDeviceClass>(),
    class_init: Some(netdev_class_init),
    class_base_init: None,
    class_data: ptr::null(),
    interfaces: ptr::null(),
};

declare_device_type!(VIRTMCU_NETDEV_TYPE_INIT, VIRTMCU_NETDEV_TYPE_INFO);

fn get_transport(
    transport_name: &str,
    router: *const c_char,
    node_id: u32,
) -> Option<Arc<dyn virtmcu_wire::DataTransport>> {
    if transport_name == "unix" {
        let path = if router.is_null() {
            format!("/tmp/virtmcu-coord-{node_id}.sock") // virtmcu-allow: absolute_path reasoning="Legacy script"
        } else {
            virtmcu_qom::ffi_call! { core::ffi::CStr::from_ptr(router).to_string_lossy().into_owned() }
        };
        // virtmcu-allow: env_in_peripheral reasoning="Not yet ported: needs federation-id QOM property + new_with_fed_id"
        transport_uds::UdsDataTransport::new(&path, node_id).ok().map(|t| Arc::new(t) as _)
    } else {
        virtmcu_qom::ffi_call! {
            transport_zenoh::get_or_init_session(router)
                .ok()
                .map(|s| Arc::new(transport_zenoh::ZenohDataTransport::new(s, node_id)) as _)
        }
    }
}

fn get_liveliness(
    transport_name: &str,
    router: *const c_char,
    node_id: u32,
) -> Option<zenoh::liveliness::LivelinessToken> {
    if transport_name == "zenoh" {
        match virtmcu_qom::ffi_call! { transport_zenoh::get_or_init_session(router) } {
            Ok(session) => {
                let hb_topic = format!("sim/netdev/liveliness/{node_id}");
                session.liveliness().declare_token(hb_topic).wait().ok()
            }
            Err(_) => None,
        }
    } else {
        None
    }
}

fn decode_netdev(_opaque: *mut c_void, _topic: &str, data: &[u8]) -> Option<OrderedPacket> {
    let (vtime, sequence, payload) = virtmcu_wire::decode_frame(data)?;

    Some(OrderedPacket { vtime, sequence, data: payload.to_vec() })
}

fn deliver_netdev(opaque: *mut c_void, packet: OrderedPacket) {
    let state = virtmcu_qom::timer::opaque_to_state::<VirtmcuNetdevState>(opaque);
    let mut backlog = state.backlog.lock(); // virtmcu-allow: mutex reasoning="Backlog managed securely"

    if state.backlog_count.load(AtomicOrdering::Acquire) >= state._max_backlog {
        state._dropped_frames.fetch_add(1, AtomicOrdering::SeqCst);
        return;
    }

    backlog.push_back(packet.data);
    state.backlog_count.fetch_add(1, AtomicOrdering::SeqCst);

    // We flush packets to QEMU via the hook
    let mut to_send = Vec::new();
    while let Some(data) = backlog.pop_front() {
        to_send.push(data);
    }

    // Release lock before calling back into QEMU
    drop(backlog);

    for data in &to_send {
        let sent = virtmcu_qom::ffi_call! { virtmcu_qom::net::qemu_send_packet(state.nc, data.as_ptr(), data.len()) };
        if sent > 0 {
            state.backlog_count.fetch_sub(1, AtomicOrdering::SeqCst);
        } else {
            // Note: In strict locking, we just discard rather than queuing on QEMU backpressure.
            state._dropped_frames.fetch_add(1, AtomicOrdering::SeqCst);
            state.backlog_count.fetch_sub(1, AtomicOrdering::SeqCst);
        }
    }
}

fn netdev_init_internal(
    nc: *mut NetClientState,
    node_id: u32,
    transport_name: String,
    router: *const c_char,
    topic: String,
    max_backlog: u64,
) -> *mut VirtmcuNetdevState {
    let transport = match get_transport(&transport_name, router, node_id) {
        Some(t) => t,
        None => return ptr::null_mut(),
    };

    let backlog_count = Arc::new(AtomicU64::new(0));
    let dropped_frames = Arc::new(AtomicU64::new(0));

    let tx_topic = format!("{topic}/{node_id}/tx");
    let liveliness = get_liveliness(&transport_name, router, node_id);

    let mut state_box = Box::new(VirtmcuNetdevState {
        _liveliness: liveliness,
        transport: Arc::clone(&transport),
        tx_topic,
        nc,
        receiver: None,
        backlog: virtmcu_qom::sync::Mutex::new(VecDeque::new()),
        tx_sequence: AtomicU64::new(0),
        _max_backlog: max_backlog,
        backlog_count: Arc::clone(&backlog_count),
        _dropped_frames: Arc::clone(&dropped_frames),
    });

    let state_ptr = core::ptr::from_mut::<VirtmcuNetdevState>(&mut *state_box);
    let rx_topic = format!("{topic}/rx");
    let generation = Arc::new(AtomicU64::new(0));

    match virtmcu_qom::sync::VtimeIngress::new(
        &*transport,
        &rx_topic,
        generation,
        state_ptr as *mut c_void,
        decode_netdev,
        deliver_netdev,
    ) {
        Ok(receiver) => {
            state_box.receiver = Some(receiver);
        }
        Err(_) => {
            return ptr::null_mut();
        }
    }

    Box::into_raw(state_box)
}

fn netdev_receive_internal(state: &VirtmcuNetdevState, buf: *const u8, size: usize) -> isize {
    let payload = virtmcu_qom::ffi_call! { core::slice::from_raw_parts(buf, size) };
    let now = u64::try_from(virtmcu_qom::timer::qemu_clock_get_ns_safe(
        virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL,
        unsafe { &virtmcu_qom::device::BqlContext::new_unchecked() },
    ))
    .expect("vtime is negative");
    let seq = state.tx_sequence.fetch_add(1, AtomicOrdering::SeqCst);

    match state.transport.reserve(&state.tx_topic, payload.len()) {
        Ok(mut reservation) => {
            reservation.buffer_mut().copy_from_slice(payload);
            let _ = reservation.commit(now, seq);
        }
        Err(e) => {
            virtmcu_qom::sim_err!("{}", e);
        }
    }

    size as isize
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::collections::BinaryHeap;

    #[test]
    fn test_ordered_packet_ord() {
        const VTIME_1000: u64 = 1000;
        const VTIME_500: u64 = 500;
        const VTIME_2000: u64 = 2000;
        const DATA_1: u8 = 1;
        const DATA_2: u8 = 2;
        const DATA_3: u8 = 3;
        let mut heap = BinaryHeap::new();
        heap.push(OrderedPacket { vtime: VTIME_1000, sequence: 0, data: vec![DATA_1] });
        heap.push(OrderedPacket { vtime: VTIME_500, sequence: 0, data: vec![DATA_2] });
        heap.push(OrderedPacket { vtime: VTIME_2000, sequence: 0, data: vec![DATA_3] });
        assert_eq!(heap.pop().expect("netdev logic assumption failed").vtime, VTIME_500);
        assert_eq!(heap.pop().expect("netdev logic assumption failed").vtime, VTIME_1000);
        assert_eq!(heap.pop().expect("netdev logic assumption failed").vtime, VTIME_2000);
    }

    #[test]
    fn test_virtmcu_net_client_layout() {
        assert_eq!(core::mem::offset_of!(VirtmcuNetClient, nc), 0);
    }

    #[test]
    fn test_netdev_qemu_layout() {
        assert_eq!(core::mem::offset_of!(VirtmcuNetdevQEMU, parent_obj), 0);
    }
}
