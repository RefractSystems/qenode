//! VirtMCU virtual network device with pluggable transport.
use zenoh::Wait;

extern crate alloc;

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ffi::{c_char, c_int, c_void, CStr};
use core::ptr;
use core::time::Duration;
use virtmcu_qom::error::Error;
use virtmcu_qom::net::{
    qemu_new_net_client, virtmcu_netdev_hook, NetClientInfo, NetClientState, Netdev,
    NET_CLIENT_DRIVER_VIRTMCU,
};
use virtmcu_qom::qdev::SysBusDevice;
use virtmcu_qom::qom::{ObjectClass, TypeInfo};
use virtmcu_qom::sync::{Bql, BqlGuarded, SafeSubscription}; // BQL_EXCEPTION: Safe Zenoh integration
use virtmcu_qom::{declare_device_type, device_class, error_setg};

use alloc::collections::{BinaryHeap, VecDeque};
use core::cmp::Ordering;
use core::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use crossbeam_channel::{bounded, Receiver, Sender, TrySendError};
use std::sync::{Condvar, Mutex};
use virtmcu_api::{FlatBufferStructExt, ZenohFrameHeader};
use virtmcu_qom::timer::{qemu_clock_get_ns, QomTimer, QEMU_CLOCK_VIRTUAL};

#[repr(C)]
pub struct VirtmcuNetdevQEMU {
    pub parent_obj: SysBusDevice,
    pub nc: NetClientState,
    pub rust_state: *mut VirtmcuNetdevState,
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

pub struct TxPacket {
    pub vtime: u64,
    pub sequence: u64,
    pub data: Vec<u8>,
}

pub struct VirtmcuNetdevState {
    shared: Arc<SharedState>,
    nc: *mut NetClientState,
    subscription: Option<SafeSubscription>, // BQL_EXCEPTION: Safe Zenoh integration
    rx_timer: Option<Arc<QomTimer>>,
    rx_receiver: Receiver<OrderedPacket>,
    // All state accessed exclusively under BQL; see BqlGuarded docs.
    local_heap: BqlGuarded<BinaryHeap<OrderedPacket>>,
    backlog: BqlGuarded<VecDeque<Vec<u8>>>,
    earliest_vtime: Arc<AtomicU64>,
    tx_sequence: AtomicU64,
    tx_thread: Option<std::thread::JoinHandle<()>>,
    _max_backlog: u64,
    backlog_count: Arc<AtomicU64>,
    _dropped_frames: Arc<AtomicU64>,
    pub _liveliness: Option<zenoh::liveliness::LivelinessToken>,
}

struct InnerState {
    running: bool,
    active_vcpu_count: usize,
}

struct SharedState {
    transport: Arc<dyn virtmcu_api::DataTransport>,
    _node_id: u32,
    _topic: String,
    tx_sender: Sender<TxPacket>,
    drain_cond: Condvar,
    state: Mutex<InnerState>, // MUTEX_EXCEPTION: used for lifecycle
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
    let backlog = unsafe { (*s.rust_state).backlog.get() };
    backlog.is_empty()
}

unsafe extern "C" fn netdev_cleanup(nc: *mut NetClientState) {
    let s = unsafe { &mut *(nc as *mut VirtmcuNetClient) };
    if !s.rust_state.is_null() {
        unsafe {
            let mut state = Box::from_raw(s.rust_state);
            {
                let mut lock =
                    state.shared.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                lock.running = false;
            }

            state.subscription.take();
            state.rx_timer.take();

            let mut lock =
                state.shared.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            while lock.active_vcpu_count > 0 {
                let bql_unlock = Bql::temporary_unlock();
                let (new_lock, timed_out) = state
                    .shared
                    .drain_cond
                    .wait_timeout(lock, Duration::from_secs(30))
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                lock = new_lock;
                drop(bql_unlock);
                if timed_out.timed_out() {
                    break;
                }
            }

            if let Some(handle) = state.tx_thread.take() {
                let bql_unlock = Bql::temporary_unlock();
                let _ = handle.join();
                drop(bql_unlock);
            }

            s.rust_state = ptr::null_mut();
        }
    }
}

struct VcpuCountGuard<'a>(&'a SharedState);
impl Drop for VcpuCountGuard<'_> {
    fn drop(&mut self) {
        let mut lock = self.0.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        lock.active_vcpu_count = lock.active_vcpu_count.saturating_sub(1);
        if lock.active_vcpu_count == 0 {
            self.0.drain_cond.notify_all();
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
    _opaque: [0; 208 - 56],
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
        unsafe { CStr::from_ptr(opts.node) }.to_string_lossy().parse::<u32>().unwrap_or(0)
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

fn drain_net_backlog(state: &VirtmcuNetdevState) -> bool {
    let mut backlog = state.backlog.get_mut();
    while let Some(_packet) = backlog.front() {
        if unsafe { !virtmcu_qom::net::qemu_can_receive_packet(state.nc) } {
            return false;
        }
        let data = backlog.pop_front().unwrap_or_else(|| std::process::abort());
        state.backlog_count.fetch_sub(1, AtomicOrdering::SeqCst);
        unsafe {
            virtmcu_qom::net::qemu_send_packet(state.nc, data.as_ptr(), data.len());
        }
    }
    true
}

extern "C" fn rx_timer_cb(opaque: *mut core::ffi::c_void) {
    debug_assert!(Bql::is_held(), "BQL must be held during timer callbacks");
    let state = unsafe { &*(opaque as *mut VirtmcuNetdevState) };
    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;

    if !drain_net_backlog(state) {
        if let Some(rx_timer) = &state.rx_timer {
            rx_timer.mod_ns(now as i64 + 1_000_000); // 1ms
        }
        return;
    }

    let mut heap = state.local_heap.get_mut();
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
            state.backlog_count.fetch_sub(1, AtomicOrdering::SeqCst);
            unsafe {
                virtmcu_qom::net::qemu_send_packet(state.nc, p.data.as_ptr(), p.data.len());
            }
        } else {
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

fn start_tx_thread(
    shared: Arc<SharedState>,
    rx_out: Receiver<TxPacket>,
) -> std::thread::JoinHandle<()> {
    let shared_clone = Arc::clone(&shared);
    std::thread::spawn(move || loop {
        {
            let lock = shared_clone.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if !lock.running && rx_out.is_empty() {
                break;
            }
        }
        match rx_out.recv_timeout(Duration::from_millis(10)) {
            Ok(packet) => {
                let header =
                    ZenohFrameHeader::new(packet.vtime, packet.sequence, packet.data.len() as u32);
                let mut data =
                    Vec::with_capacity(virtmcu_api::ZENOH_FRAME_HEADER_SIZE + packet.data.len());
                data.extend_from_slice(header.pack());
                data.extend_from_slice(&packet.data);

                if let Err(e) = shared_clone.transport.publish(&shared_clone._topic, &data) {
                    virtmcu_qom::sim_err!("{}", e);
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
        }
    })
}

fn get_transport(
    transport_name: &str,
    router: *const c_char,
    node_id: u32,
) -> Option<Arc<dyn virtmcu_api::DataTransport>> {
    if transport_name == "unix" {
        let path = if router.is_null() {
            format!("/tmp/virtmcu-coord-{node_id}.sock")
        } else {
            unsafe { core::ffi::CStr::from_ptr(router).to_string_lossy().into_owned() }
        };
        transport_unix::UnixDataTransport::new(&path).ok().map(|t| Arc::new(t) as _)
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

    let (tx, rx) = bounded(65536);
    let (tx_out, rx_out) = bounded(65536);
    let local_heap = BqlGuarded::new(BinaryHeap::new());
    let earliest_vtime = Arc::new(AtomicU64::new(u64::MAX));

    let backlog_count = Arc::new(AtomicU64::new(0));
    let dropped_frames = Arc::new(AtomicU64::new(0));
    let backlog_count_sub = Arc::clone(&backlog_count);
    let dropped_frames_sub = Arc::clone(&dropped_frames);

    let shared = Arc::new(SharedState {
        transport,
        _node_id: node_id,
        _topic: format!("{topic}/{node_id}/tx"),
        tx_sender: tx_out,
        drain_cond: Condvar::new(),
        state: Mutex::new(InnerState { running: true, active_vcpu_count: 0 }),
    });

    let tx_thread = start_tx_thread(Arc::clone(&shared), rx_out);
    let liveliness = get_liveliness(&transport_name, router, node_id);

    let mut state = Box::new(VirtmcuNetdevState {
        _liveliness: liveliness,
        shared: Arc::clone(&shared),
        nc,
        subscription: None,
        rx_timer: None,
        rx_receiver: rx,
        local_heap,
        backlog: BqlGuarded::new(VecDeque::new()),
        earliest_vtime,
        tx_sequence: AtomicU64::new(0),
        tx_thread: Some(tx_thread),
        _max_backlog: max_backlog,
        backlog_count: Arc::clone(&backlog_count),
        _dropped_frames: Arc::clone(&dropped_frames),
    });

    let state_ptr = core::ptr::from_mut::<VirtmcuNetdevState>(&mut *state);
    let rx_timer = Arc::new(unsafe {
        QomTimer::new(QEMU_CLOCK_VIRTUAL, rx_timer_cb, state_ptr as *mut c_void)
    });
    let rx_timer_clone = Arc::clone(&rx_timer);
    let earliest_clone = Arc::clone(&state.earliest_vtime);

    let rx_topic = format!("{topic}/rx");
    let sub_callback: virtmcu_api::DataCallback = Box::new(move |data| {
        if data.len() < virtmcu_api::ZENOH_FRAME_HEADER_SIZE {
            return;
        }
        let header =
            match ZenohFrameHeader::unpack_slice(&data[..virtmcu_api::ZENOH_FRAME_HEADER_SIZE]) {
                Some(h) => h,
                None => return,
            };
        let payload = data[virtmcu_api::ZENOH_FRAME_HEADER_SIZE..].to_vec();
        let packet = OrderedPacket {
            vtime: header.delivery_vtime_ns(),
            sequence: header.sequence_number(),
            data: payload,
        };
        if backlog_count_sub.load(AtomicOrdering::Acquire) >= max_backlog {
            dropped_frames_sub.fetch_add(1, AtomicOrdering::SeqCst);
            return;
        }
        if tx.try_send(packet).is_ok() {
            backlog_count_sub.fetch_add(1, AtomicOrdering::SeqCst);
            let current_earliest = earliest_clone.load(AtomicOrdering::Acquire);
            if header.delivery_vtime_ns() < current_earliest {
                earliest_clone.fetch_min(header.delivery_vtime_ns(), AtomicOrdering::Release);
                rx_timer_clone.mod_ns(header.delivery_vtime_ns() as i64);
            }
        } else {
            dropped_frames_sub.fetch_add(1, AtomicOrdering::SeqCst);
        }
    });

    let generation = Arc::new(AtomicU64::new(0));
    state.subscription =
        SafeSubscription::new(&*shared.transport, &rx_topic, generation, sub_callback).ok(); // BQL_EXCEPTION: Safe Zenoh integration

    state.rx_timer = Some(rx_timer);

    Box::into_raw(state)
}

fn netdev_receive_internal(state: &VirtmcuNetdevState, buf: *const u8, size: usize) -> isize {
    {
        let mut lock = state.shared.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if !lock.running {
            return 0;
        }
        lock.active_vcpu_count += 1;
    }
    let _guard = VcpuCountGuard(&state.shared);
    let payload = unsafe { core::slice::from_raw_parts(buf, size) };
    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;
    let seq = state.tx_sequence.fetch_add(1, AtomicOrdering::SeqCst);

    match state.shared.tx_sender.try_send(TxPacket {
        vtime: now,
        sequence: seq,
        data: payload.to_vec(),
    }) {
        Ok(_) | Err(TrySendError::Disconnected(_) | TrySendError::Full(_)) => {}
    }
    size as isize
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::collections::BinaryHeap;

    #[test]
    fn test_ordered_packet_ord() {
        let mut heap = BinaryHeap::new();
        heap.push(OrderedPacket { vtime: 1000, sequence: 0, data: vec![1] });
        heap.push(OrderedPacket { vtime: 500, sequence: 0, data: vec![2] });
        heap.push(OrderedPacket { vtime: 2000, sequence: 0, data: vec![3] });
        assert_eq!(heap.pop().unwrap().vtime, 500);
        assert_eq!(heap.pop().unwrap().vtime, 1000);
        assert_eq!(heap.pop().unwrap().vtime, 2000);
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
