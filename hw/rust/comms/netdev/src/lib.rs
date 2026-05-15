#![allow(clippy::panic)] // virtmcu-allow: allow reasoning="Fail Loudly"
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
use core::time::Duration;
use crossbeam_channel::{bounded, Receiver, Sender, TrySendError};
use virtmcu_api::{FlatBufferStructExt, ZenohFrameHeader};
use virtmcu_qom::cosim::{CoSimBridge, CoSimContext, CoSimTransport};
use virtmcu_qom::error::Error;
use virtmcu_qom::net::{
    qemu_new_net_client, virtmcu_netdev_hook, NetClientInfo, NetClientState, Netdev,
    NET_CLIENT_DRIVER_VIRTMCU,
};
use virtmcu_qom::qdev::SysBusDevice;
use virtmcu_qom::qom::{ObjectClass, TypeInfo};
use virtmcu_qom::timer::{qemu_clock_get_ns, QEMU_CLOCK_VIRTUAL};
use virtmcu_qom::{declare_device_type, device_class, error_setg};

const NETDEV_INFO_OPAQUE_SIZE: usize = 208 - 56;
const RECV_TIMEOUT_MS: u64 = 10;
const TX_QUEUE_SIZE: usize = 65536;

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

pub struct TxPacket {
    pub vtime: u64,
    pub sequence: u64,
    pub data: Vec<u8>,
}

pub struct NetdevTransport {
    pub transport: Arc<dyn virtmcu_api::DataTransport>,
    pub topic: String,
    pub tx_sender: Sender<TxPacket>,
    pub rx_out: Receiver<TxPacket>,
}
unsafe impl Send for NetdevTransport {}
unsafe impl Sync for NetdevTransport {}

impl CoSimTransport for NetdevTransport {
    type Request = TxPacket;
    type Response = ();

    fn run_rx_loop(&self, ctx: &CoSimContext<Self::Response>) {
        while ctx.is_running() {
            match self.rx_out.recv_timeout(Duration::from_millis(RECV_TIMEOUT_MS)) {
                Ok(packet) => {
                    let header = ZenohFrameHeader::new(
                        packet.vtime,
                        packet.sequence,
                        u32::try_from(packet.data.len()).expect("payload length truncated"),
                    );
                    let mut data = Vec::with_capacity(
                        virtmcu_api::ZENOH_FRAME_HEADER_SIZE + packet.data.len(),
                    );
                    data.extend_from_slice(header.pack());
                    data.extend_from_slice(&packet.data);

                    if let Err(e) = self.transport.publish(&self.topic, &data) {
                        virtmcu_qom::sim_err!("{}", e);
                    }
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            }
        }
    }

    fn send_request(&self, req: Self::Request) -> bool {
        match self.tx_sender.try_send(req) {
            Ok(_) | Err(TrySendError::Disconnected(_) | TrySendError::Full(_)) => {}
        }
        false
    }

    fn interrupt_rx(&self) {}
}

pub struct VirtmcuNetdevState {
    bridge: CoSimBridge<NetdevTransport>,
    nc: *mut NetClientState,
    receiver: Option<virtmcu_qom::sync::DeterministicReceiver<OrderedPacket>>,
    backlog: virtmcu_qom::sync::Mutex<VecDeque<Vec<u8>>>, // virtmcu-allow: mutex reasoning="Backlog managed securely"
    tx_sequence: AtomicU64,
    _max_backlog: u64,
    backlog_count: Arc<AtomicU64>,
    _dropped_frames: Arc<AtomicU64>,
    pub _liveliness: Option<zenoh::liveliness::LivelinessToken>,
}

unsafe extern "C" fn netdev_receive(nc: *mut NetClientState, buf: *const u8, size: usize) -> isize {
    let s = unsafe { &mut *(nc as *mut VirtmcuNetClient) };
    if s.rust_state.is_null() {
        return 0;
    }
    unsafe { netdev_receive_internal(&*s.rust_state, buf, size) }
}

unsafe extern "C" fn netdev_can_receive(nc: *mut NetClientState) -> bool {
    let s = unsafe { &mut *(nc as *mut VirtmcuNetClient) };
    if s.rust_state.is_null() {
        return true;
    }
    let backlog = unsafe { (*s.rust_state).backlog.lock() };
    backlog.is_empty()
}

unsafe extern "C" fn netdev_cleanup(nc: *mut NetClientState) {
    let s = unsafe { &mut *(nc as *mut VirtmcuNetClient) };
    if !s.rust_state.is_null() {
        unsafe {
            let mut state = Box::from_raw(s.rust_state);
            state.receiver.take();
            // Drop handles bridge teardown automatically
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

unsafe extern "C" fn netdev_hook(
    netdev: *const Netdev,
    name: *const c_char,
    peer: *mut NetClientState,
    errp: *mut *mut Error,
) -> c_int {
    let opts = unsafe { &(*netdev).u.virtmcu };

    let nc = unsafe {
        qemu_new_net_client(&raw const NET_VIRTMCU_INFO, peer, c"virtmcu".as_ptr(), name)
    };
    let s = unsafe { &mut *(nc as *mut VirtmcuNetClient) };

    let node_id = if opts.node.is_null() {
        0
    } else {
        unsafe { CStr::from_ptr(opts.node) }
            .to_string_lossy()
            .parse::<u32>()
            .expect("Invalid data format")
    };

    let transport_name = if opts.transport.is_null() {
        "zenoh".to_owned()
    } else {
        unsafe { CStr::from_ptr(opts.transport) }.to_string_lossy().into_owned()
    };

    let router = if opts.router.is_null() { ptr::null() } else { opts.router.cast_const() };

    let topic = if opts.topic.is_null() {
        "sim/eth/frame".to_owned()
    } else {
        unsafe { CStr::from_ptr(opts.topic) }.to_string_lossy().into_owned()
    };

    let max_backlog = if opts.has_max_backlog { opts.max_backlog } else { 256 };

    s.rust_state = netdev_init_internal(nc, node_id, transport_name, router, topic, max_backlog);
    if s.rust_state.is_null() {
        error_setg!(errp, "netdev: failed to initialize Rust backend");
        return -1;
    }

    0
}

unsafe extern "C" fn netdev_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let dc = device_class!(klass);
    unsafe {
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
) -> Option<Arc<dyn virtmcu_api::DataTransport>> {
    if transport_name == "unix" {
        let path = if router.is_null() {
            format!("/tmp/virtmcu-coord-{node_id}.sock") // virtmcu-allow: absolute_path reasoning="Legacy script"
        } else {
            unsafe { core::ffi::CStr::from_ptr(router).to_string_lossy().into_owned() }
        };
        transport_unix::UdsDataTransport::new(&path).ok().map(|t| Arc::new(t) as _)
    } else {
        unsafe {
            transport_zenoh::get_or_init_session(router)
                .ok()
                .map(|s| Arc::new(transport_zenoh::ZenohDataTransport::new(s)) as _)
        }
    }
}

fn get_liveliness(
    transport_name: &str,
    router: *const c_char,
    node_id: u32,
) -> Option<zenoh::liveliness::LivelinessToken> {
    if transport_name == "zenoh" {
        match unsafe { transport_zenoh::get_or_init_session(router) } {
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
    if data.len() < virtmcu_api::ZENOH_FRAME_HEADER_SIZE {
        return None;
    }
    let header_slice = data.get(..virtmcu_api::ZENOH_FRAME_HEADER_SIZE)?;
    let header = ZenohFrameHeader::unpack(header_slice.try_into().ok()?)?;
    let payload = data.get(virtmcu_api::ZENOH_FRAME_HEADER_SIZE..)?.to_vec();
    Some(OrderedPacket {
        vtime: header.delivery_vtime_ns(),
        sequence: header.sequence_number(),
        data: payload,
    })
}

fn deliver_netdev(opaque: *mut c_void, packet: OrderedPacket) {
    let state = unsafe { &mut *(opaque as *mut VirtmcuNetdevState) };
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
        let sent =
            unsafe { virtmcu_qom::net::qemu_send_packet(state.nc, data.as_ptr(), data.len()) };
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

    let (tx_out, rx_out) = bounded(TX_QUEUE_SIZE);

    let backlog_count = Arc::new(AtomicU64::new(0));
    let dropped_frames = Arc::new(AtomicU64::new(0));

    let netdev_transport = NetdevTransport {
        transport: Arc::clone(&transport),
        topic: format!("{topic}/{node_id}/tx"),
        tx_sender: tx_out,
        rx_out,
    };
    let bridge = CoSimBridge::new(netdev_transport);

    let liveliness = get_liveliness(&transport_name, router, node_id);

    let mut state_box = Box::new(VirtmcuNetdevState {
        _liveliness: liveliness,
        bridge,
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

    match virtmcu_qom::sync::DeterministicReceiver::new(
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
    let payload = unsafe { core::slice::from_raw_parts(buf, size) };
    let now =
        u64::try_from(unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) }).expect("vtime is negative");
    let seq = state.tx_sequence.fetch_add(1, AtomicOrdering::SeqCst);

    state.bridge.send_and_wait(TxPacket { vtime: now, sequence: seq, data: payload.to_vec() }, 0);
    size as isize
}

#[cfg(test)]
#[allow(clippy::magic_numbers)] // virtmcu-allow: allow reasoning="Tests require specific magic numbers"
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
