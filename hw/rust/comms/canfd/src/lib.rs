//! Virtmcu virtual CAN FD device with pluggable transport.
use zenoh::Wait;

extern crate alloc;

use alloc::boxed::Box;
use alloc::collections::{BinaryHeap, VecDeque};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cmp::Ordering;
use core::ffi::{c_char, c_void, CStr};
use core::ptr;
use core::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use crossbeam_channel::{unbounded, Receiver, Sender};
use flatbuffers::root;
use virtmcu_api::can_generated::virtmcu::can::{CanFdFrame, CanFdFrameArgs};
use virtmcu_qom::declare_device_type;
use virtmcu_qom::error::Error;
use virtmcu_qom::net::{
    can_bus_client_send, can_bus_insert_client, can_bus_remove_client, CanBusClientInfo,
    CanBusClientState, CanHostClass, CanHostState, QemuCanFrame,
};
use virtmcu_qom::qom::{Object, ObjectClass, TypeInfo};
use virtmcu_qom::sync::{BqlGuarded, SafeSubscription}; // BQL_EXCEPTION: Safe Zenoh integration
use virtmcu_qom::timer::{qemu_clock_get_ns, QomTimer, QEMU_CLOCK_VIRTUAL};

pub const TYPE_CAN_HOST_VIRTMCU: *const c_char = c"can-host-virtmcu".as_ptr();

#[repr(C)]
pub struct VirtmcuCanHostState {
    pub parent_obj: CanHostState,
    pub node: *mut c_char,
    pub transport: *mut c_char,
    pub router: *mut c_char,
    pub topic: *mut c_char,
    pub rust_state: *mut State,
}

pub struct OrderedCanFrame {
    pub vtime: u64,
    pub sequence: u64,
    pub frame: QemuCanFrame,
}

impl PartialEq for OrderedCanFrame {
    fn eq(&self, other: &Self) -> bool {
        self.vtime == other.vtime && self.sequence == other.sequence
    }
}
impl Eq for OrderedCanFrame {}
impl PartialOrd for OrderedCanFrame {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for OrderedCanFrame {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse for min-heap
        match other.vtime.cmp(&self.vtime) {
            Ordering::Equal => other.sequence.cmp(&self.sequence),
            ord => ord,
        }
    }
}

pub struct State {
    transport: Arc<dyn virtmcu_api::DataTransport>,
    subscription: Option<SafeSubscription>, // BQL_EXCEPTION: Safe Zenoh integration
    tx_sender: Sender<Vec<u8>>,
    rx_sender: Sender<OrderedCanFrame>,
    rx_receiver: Receiver<OrderedCanFrame>,
    local_heap: BqlGuarded<BinaryHeap<OrderedCanFrame>>,
    backlog: BqlGuarded<VecDeque<QemuCanFrame>>,
    earliest_vtime: Arc<AtomicU64>,
    rx_timer: Option<Arc<QomTimer>>,
    client_ptr: *mut CanBusClientState,
    tx_sequence: AtomicU64,
    _topic: String,
    pub _liveliness: Option<zenoh::liveliness::LivelinessToken>,
}

unsafe extern "C" fn virtmcu_can_receive(client: *mut CanBusClientState) -> bool {
    let ch = unsafe { (*client).peer as *mut VirtmcuCanHostState };
    let state = unsafe { (*ch).rust_state };
    if state.is_null() {
        return true;
    }
    let backlog = unsafe { (*state).backlog.get() };
    backlog.is_empty()
}

unsafe extern "C" fn virtmcu_can_receive_frames(
    client: *mut CanBusClientState,
    frames: *const QemuCanFrame,
    frames_cnt: usize,
) -> isize {
    if frames_cnt == 0 {
        return 0;
    }

    let ch = unsafe { (*client).peer as *mut VirtmcuCanHostState };
    let state = unsafe { (*ch).rust_state };
    if state.is_null() {
        return frames_cnt as isize;
    }

    let slice = unsafe { core::slice::from_raw_parts(frames, frames_cnt) };
    let vtime_ns = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };

    for frame in slice {
        let mut builder = flatbuffers::FlatBufferBuilder::new();
        let data_vec = builder.create_vector(&frame.data[..frame.can_dlc as usize]);
        let seq = unsafe { (*state).tx_sequence.fetch_add(1, AtomicOrdering::SeqCst) };
        let fbs_frame = CanFdFrame::create(
            &mut builder,
            &CanFdFrameArgs {
                delivery_vtime_ns: vtime_ns as u64,
                sequence_number: seq,
                can_id: frame.can_id,
                flags: u32::from(frame.flags),
                data: Some(data_vec),
            },
        );
        builder.finish(fbs_frame, None);
        let payload = builder.finished_data().to_vec();
        let _ = unsafe { (*state).tx_sender.send(payload) };
    }

    frames_cnt as isize
}

static VIRTMCU_CAN_CLIENT_INFO: CanBusClientInfo = CanBusClientInfo {
    can_receive: Some(virtmcu_can_receive),
    receive: Some(virtmcu_can_receive_frames),
};

fn drain_can_backlog(state: &State) -> bool {
    let mut backlog = state.backlog.get_mut();
    while let Some(_frame) = backlog.front() {
        if unsafe {
            match (*(*state.client_ptr).info).can_receive {
                Some(can_receive) => !can_receive(state.client_ptr),
                None => false,
            }
        } {
            return false;
        }

        let f = backlog.pop_front().unwrap_or_else(|| std::process::abort());
        unsafe {
            can_bus_client_send(state.client_ptr, &raw const f, 1);
        }
    }
    true
}

extern "C" fn rx_timer_cb(opaque: *mut core::ffi::c_void) {
    let state = unsafe { &*(opaque as *mut State) };
    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;

    if !drain_can_backlog(state) {
        if let Some(rx_timer) = &state.rx_timer {
            rx_timer.mod_ns(now as i64 + 1_000_000);
        }
        return;
    }

    let mut heap = state.local_heap.get_mut();

    while let Ok(packet) = state.rx_receiver.try_recv() {
        heap.push(packet);
    }
    while let Some(packet) = heap.peek() {
        if packet.vtime <= now {
            if unsafe {
                match (*(*state.client_ptr).info).can_receive {
                    Some(can_receive) => !can_receive(state.client_ptr),
                    None => false,
                }
            } {
                let mut backlog = state.backlog.get_mut();
                let p = heap.pop().unwrap_or_else(|| std::process::abort());
                backlog.push_back(p.frame);
                break;
            }

            let p = heap.pop().unwrap_or_else(|| std::process::abort());
            unsafe {
                can_bus_client_send(state.client_ptr, &raw const p.frame, 1);
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

fn create_can_sub_callback(
    tx_clone: Sender<OrderedCanFrame>,
    earliest_vtime: Arc<AtomicU64>,
    rx_timer_clone: Arc<QomTimer>,
) -> virtmcu_api::DataCallback {
    Box::new(move |data| {
        if let Ok(fbs) = root::<CanFdFrame>(data) {
            let mut data_arr = [0u8; 64];
            let dlc = if let Some(d) = fbs.data() {
                let len = core::cmp::min(d.len(), 64);
                data_arr[..len].copy_from_slice(&d.bytes()[..len]);
                len as u8
            } else {
                0
            };

            let frame = QemuCanFrame {
                can_id: fbs.can_id(),
                can_dlc: dlc,
                flags: fbs.flags() as u8,
                _padding: [0; 2],
                data: data_arr,
            };

            let vtime = fbs.delivery_vtime_ns();
            let sequence = fbs.sequence_number();
            let packet = OrderedCanFrame { vtime, sequence, frame };

            if tx_clone.send(packet).is_ok() {
                let mut current = earliest_vtime.load(AtomicOrdering::Relaxed);
                while vtime < current {
                    if earliest_vtime
                        .compare_exchange_weak(
                            current,
                            vtime,
                            AtomicOrdering::Release,
                            AtomicOrdering::Relaxed,
                        )
                        .is_ok()
                    {
                        rx_timer_clone.mod_ns(vtime as i64);
                        break;
                    }
                    current = earliest_vtime.load(AtomicOrdering::Relaxed);
                }
            }
        }
    })
}

unsafe extern "C" fn virtmcu_can_host_connect(ch: *mut CanHostState, _errp: *mut *mut Error) {
    let zch = ch as *mut VirtmcuCanHostState;

    if unsafe { (*zch).node.is_null() || (*zch).topic.is_null() } {
        return;
    }

    let node = unsafe { CStr::from_ptr((*zch).node).to_string_lossy().into_owned() };
    let topic_str = unsafe { CStr::from_ptr((*zch).topic).to_string_lossy().into_owned() };
    let transport_name = if (*zch).transport.is_null() {
        "zenoh".to_owned()
    } else {
        unsafe { CStr::from_ptr((*zch).transport).to_string_lossy().into_owned() }
    };

    let router_ptr = unsafe {
        if (*zch).router.is_null() {
            ptr::null()
        } else {
            (*zch).router.cast_const()
        }
    };

    let transport: Arc<dyn virtmcu_api::DataTransport> = if transport_name == "unix" {
        let path = if router_ptr.is_null() {
            format!("/tmp/virtmcu-coord-{node}.sock")
        } else {
            unsafe { core::ffi::CStr::from_ptr(router_ptr).to_string_lossy().into_owned() }
        };
        match transport_unix::UnixDataTransport::new(&path) {
            Ok(t) => Arc::new(t),
            Err(_) => return,
        }
    } else {
        let session = match unsafe { transport_zenoh::get_or_init_session(router_ptr) } {
            Ok(s) => s,
            Err(_) => return,
        };
        Arc::new(transport_zenoh::ZenohDataTransport::new(session))
    };

    let (tx_rx, rx_rx) = unbounded::<Vec<u8>>();
    let transport_tx = Arc::clone(&transport);
    let topic_tx = topic_str.clone();
    std::thread::spawn(move || {
        while let Ok(payload) = rx_rx.recv() {
            let _ = transport_tx.publish(&topic_tx, &payload);
        }
    });

    let (tx, rx) = unbounded();
    let earliest_vtime = Arc::new(AtomicU64::new(u64::MAX));

    unsafe {
        (*zch).parent_obj.bus_client.info =
            core::ptr::from_ref::<CanBusClientInfo>(&VIRTMCU_CAN_CLIENT_INFO).cast_mut();
        (*zch).parent_obj.bus_client.peer = zch as *mut core::ffi::c_void;
    }

    let liveliness = if transport_name == "zenoh" {
        match unsafe { transport_zenoh::get_or_init_session(router_ptr) } {
            Ok(session) => {
                let hb_topic = format!("sim/canfd/liveliness/{node}");
                session.liveliness().declare_token(hb_topic).wait().ok()
            }
            Err(_) => None,
        }
    } else {
        None
    };
    let mut state = Box::new(State {
        _liveliness: liveliness,
        transport,
        subscription: None,
        tx_sender: tx_rx,
        rx_sender: tx,
        rx_receiver: rx,
        local_heap: BqlGuarded::new(BinaryHeap::new()),
        backlog: BqlGuarded::new(VecDeque::new()),
        earliest_vtime: Arc::clone(&earliest_vtime),
        rx_timer: None,
        client_ptr: unsafe { &raw mut (*zch).parent_obj.bus_client },
        tx_sequence: AtomicU64::new(0),
        _topic: topic_str.clone(),
    });

    let state_ptr = &raw mut *state;
    let rx_timer = Arc::new(unsafe {
        QomTimer::new(QEMU_CLOCK_VIRTUAL, rx_timer_cb, state_ptr as *mut core::ffi::c_void)
    });
    let rx_timer_clone = Arc::clone(&rx_timer);

    let tx_clone = state.rx_sender.clone();
    let sub_callback =
        create_can_sub_callback(tx_clone, Arc::clone(&earliest_vtime), rx_timer_clone);

    let generation = Arc::new(AtomicU64::new(0));
    state.subscription =
        SafeSubscription::new(&*state.transport, &topic_str, generation, sub_callback).ok(); // BQL_EXCEPTION: Safe Zenoh integration
    state.rx_timer = Some(rx_timer);

    unsafe {
        (*zch).rust_state = Box::into_raw(state);
        can_bus_insert_client((*zch).parent_obj.bus, &raw mut (*zch).parent_obj.bus_client);
    }
}

unsafe extern "C" fn virtmcu_can_host_disconnect(ch: *mut CanHostState) {
    let zch = ch as *mut VirtmcuCanHostState;
    unsafe {
        can_bus_remove_client(&raw mut (*zch).parent_obj.bus_client);

        if !(*zch).rust_state.is_null() {
            let mut state = Box::from_raw((*zch).rust_state);
            state.subscription.take();
            state.rx_timer.take();
            (*zch).rust_state = ptr::null_mut();
        }
    }
}

extern "C" {
    fn object_class_property_add_str(
        klass: *mut ObjectClass,
        name: *const c_char,
        get: Option<
            unsafe extern "C" fn(
                obj: *mut virtmcu_qom::qom::Object,
                errp: *mut *mut Error,
            ) -> *mut c_char,
        >,
        set: Option<
            unsafe extern "C" fn(
                obj: *mut virtmcu_qom::qom::Object,
                value: *const c_char,
                errp: *mut *mut Error,
            ),
        >,
    ) -> *mut c_void;
    fn g_strdup(s: *const c_char) -> *mut c_char;
    fn g_free(p: *mut c_void);
}

unsafe extern "C" fn get_node(
    obj: *mut virtmcu_qom::qom::Object,
    _errp: *mut *mut Error,
) -> *mut c_char {
    let zch = obj as *mut VirtmcuCanHostState;
    unsafe { g_strdup((*zch).node) }
}

unsafe extern "C" fn set_node(
    obj: *mut virtmcu_qom::qom::Object,
    value: *const c_char,
    _errp: *mut *mut Error,
) {
    let zch = obj as *mut VirtmcuCanHostState;
    unsafe {
        if !(*zch).node.is_null() {
            g_free((*zch).node as *mut c_void);
        }
        (*zch).node = g_strdup(value);
    }
}

unsafe extern "C" fn get_transport(
    obj: *mut virtmcu_qom::qom::Object,
    _errp: *mut *mut Error,
) -> *mut c_char {
    let zch = obj as *mut VirtmcuCanHostState;
    unsafe { g_strdup((*zch).transport) }
}

unsafe extern "C" fn set_transport(
    obj: *mut virtmcu_qom::qom::Object,
    value: *const c_char,
    _errp: *mut *mut Error,
) {
    let zch = obj as *mut VirtmcuCanHostState;
    unsafe {
        if !(*zch).transport.is_null() {
            g_free((*zch).transport as *mut c_void);
        }
        (*zch).transport = g_strdup(value);
    }
}

unsafe extern "C" fn get_router(
    obj: *mut virtmcu_qom::qom::Object,
    _errp: *mut *mut Error,
) -> *mut c_char {
    let zch = obj as *mut VirtmcuCanHostState;
    unsafe { g_strdup((*zch).router) }
}

unsafe extern "C" fn set_router(
    obj: *mut virtmcu_qom::qom::Object,
    value: *const c_char,
    _errp: *mut *mut Error,
) {
    let zch = obj as *mut VirtmcuCanHostState;
    unsafe {
        if !(*zch).router.is_null() {
            g_free((*zch).router as *mut c_void);
        }
        (*zch).router = g_strdup(value);
    }
}

unsafe extern "C" fn get_topic(
    obj: *mut virtmcu_qom::qom::Object,
    _errp: *mut *mut Error,
) -> *mut c_char {
    let zch = obj as *mut VirtmcuCanHostState;
    unsafe { g_strdup((*zch).topic) }
}

unsafe extern "C" fn set_topic(
    obj: *mut virtmcu_qom::qom::Object,
    value: *const c_char,
    _errp: *mut *mut Error,
) {
    let zch = obj as *mut VirtmcuCanHostState;
    unsafe {
        if !(*zch).topic.is_null() {
            g_free((*zch).topic as *mut c_void);
        }
        (*zch).topic = g_strdup(value);
    }
}

unsafe extern "C" fn virtmcu_can_host_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let chc = klass as *mut CanHostClass;
    unsafe {
        (*chc).connect = Some(virtmcu_can_host_connect);
        (*chc).disconnect = Some(virtmcu_can_host_disconnect);
    }

    unsafe {
        object_class_property_add_str(klass, c"node".as_ptr(), Some(get_node), Some(set_node));
        object_class_property_add_str(
            klass,
            c"transport".as_ptr(),
            Some(get_transport),
            Some(set_transport),
        );
        object_class_property_add_str(
            klass,
            c"router".as_ptr(),
            Some(get_router),
            Some(set_router),
        );
        object_class_property_add_str(klass, c"topic".as_ptr(), Some(get_topic), Some(set_topic));
    }
}

unsafe extern "C" fn virtmcu_can_host_instance_init(obj: *mut Object) {
    let zch = obj as *mut VirtmcuCanHostState;
    unsafe {
        (*zch).node = ptr::null_mut();
        (*zch).transport = ptr::null_mut();
        (*zch).router = ptr::null_mut();
        (*zch).topic = ptr::null_mut();
        (*zch).rust_state = ptr::null_mut();
    }
}

unsafe extern "C" fn virtmcu_can_host_instance_finalize(obj: *mut Object) {
    let zch = obj as *mut VirtmcuCanHostState;
    unsafe {
        if !(*zch).node.is_null() {
            g_free((*zch).node as *mut c_void);
        }
        if !(*zch).transport.is_null() {
            g_free((*zch).transport as *mut c_void);
        }
        if !(*zch).router.is_null() {
            g_free((*zch).router as *mut c_void);
        }
        if !(*zch).topic.is_null() {
            g_free((*zch).topic as *mut c_void);
        }
        if !(*zch).rust_state.is_null() {
            let mut state = Box::from_raw((*zch).rust_state);
            state.subscription.take();
            state.rx_timer.take();
        }
    }
}

#[used]
static VIRTMCU_CAN_HOST_TYPE_INFO: TypeInfo = TypeInfo {
    name: TYPE_CAN_HOST_VIRTMCU,
    parent: c"can-host".as_ptr(),
    instance_size: core::mem::size_of::<VirtmcuCanHostState>(),
    instance_align: core::mem::align_of::<VirtmcuCanHostState>(),
    instance_init: Some(virtmcu_can_host_instance_init),
    instance_post_init: None,
    instance_finalize: Some(virtmcu_can_host_instance_finalize),
    abstract_: false,
    class_size: core::mem::size_of::<CanHostClass>(),
    class_init: Some(virtmcu_can_host_class_init),
    class_base_init: None,
    class_data: core::ptr::null(),
    interfaces: core::ptr::null(),
};

declare_device_type!(VIRTMCU_CANFD_TYPE_INIT, VIRTMCU_CAN_HOST_TYPE_INFO);
