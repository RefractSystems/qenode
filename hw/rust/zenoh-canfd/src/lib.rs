#![allow(unused_variables)]
#![allow(clippy::all)]
#![allow(clippy::missing_safety_doc, dead_code, unused_imports)]

use core::ffi::{c_char, c_int, c_void};
use crossbeam_channel::{unbounded, Receiver, Sender};
use flatbuffers::root;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, VecDeque};
use std::ffi::{CStr, CString};
use std::ptr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};
use virtmcu_api::can_generated::virtmcu::can::{CanFdFrame, CanFdFrameArgs};
use virtmcu_qom::declare_device_type;
use virtmcu_qom::error::Error;
use virtmcu_qom::net::{
    can_bus_client_send, can_bus_insert_client, can_bus_remove_client, CanBusClientInfo,
    CanBusClientState, CanHostClass, CanHostState, QemuCanFrame,
};
use virtmcu_qom::qom::{type_register_static, Object, ObjectClass, Property, TypeInfo};
use virtmcu_qom::timer::{qemu_clock_get_ns, QomTimer, QEMU_CLOCK_VIRTUAL};
use virtmcu_zenoh::{open_session, SafeSubscriber};
use zenoh::Session;
use zenoh::Wait;

pub const TYPE_CAN_HOST_ZENOH: *const c_char = c"can-host-zenoh".as_ptr();

#[repr(C)]
pub struct ZenohCanHostState {
    pub parent_obj: CanHostState,
    pub node: *mut c_char,
    pub router: *mut c_char,
    pub topic: *mut c_char,
    pub rust_state: *mut State,
}

pub struct OrderedCanFrame {
    pub vtime: u64,
    pub frame: QemuCanFrame,
}

impl PartialEq for OrderedCanFrame {
    fn eq(&self, other: &Self) -> bool {
        self.vtime == other.vtime
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
        other.vtime.cmp(&self.vtime)
    }
}

pub struct State {
    session: Session,
    subscriber: Option<SafeSubscriber>,
    tx_sender: Sender<Vec<u8>>,
    rx_sender: Sender<OrderedCanFrame>,
    rx_receiver: Receiver<OrderedCanFrame>,
    local_heap: Mutex<BinaryHeap<OrderedCanFrame>>,
    backlog: Mutex<VecDeque<QemuCanFrame>>,
    earliest_vtime: Arc<AtomicU64>,
    rx_timer: Option<Arc<QomTimer>>,
    client_ptr: *mut CanBusClientState,
}

unsafe extern "C" fn zenoh_can_receive(client: *mut CanBusClientState) -> bool {
    let ch = (*client).peer as *mut ZenohCanHostState;
    let state = (*ch).rust_state;
    if state.is_null() {
        return true;
    }
    let backlog = (*state).backlog.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    backlog.is_empty()
}

unsafe extern "C" fn zenoh_can_receive_frames(
    client: *mut CanBusClientState,
    frames: *const QemuCanFrame,
    frames_cnt: usize,
) -> isize {
    if frames_cnt == 0 {
        return 0;
    }

    let ch = (*client).peer as *mut ZenohCanHostState;
    let state = (*ch).rust_state;
    if state.is_null() {
        return frames_cnt as isize;
    }

    let slice = std::slice::from_raw_parts(frames, frames_cnt);
    let vtime_ns = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL);

    for frame in slice {
        let mut builder = flatbuffers::FlatBufferBuilder::new();
        let data_vec = builder.create_vector(&frame.data[..frame.can_dlc as usize]);
        let fbs_frame = CanFdFrame::create(
            &mut builder,
            &CanFdFrameArgs {
                delivery_vtime_ns: vtime_ns as u64,
                can_id: frame.can_id,
                flags: u32::from(frame.flags),
                data: Some(data_vec),
            },
        );
        builder.finish(fbs_frame, None);
        let payload = builder.finished_data().to_vec();

        let _ = (*state).tx_sender.send(payload);
    }

    frames_cnt as isize
}

static mut ZENOH_CAN_CLIENT_INFO: CanBusClientInfo = CanBusClientInfo {
    can_receive: Some(zenoh_can_receive),
    receive: Some(zenoh_can_receive_frames),
};

fn drain_can_backlog(state: &State) -> bool {
    let mut backlog = state.backlog.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    while let Some(frame) = backlog.front() {
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

    let mut heap = state.local_heap.lock().unwrap_or_else(std::sync::PoisonError::into_inner);

    while let Ok(packet) = state.rx_receiver.try_recv() {
        heap.push(packet);
    }
    while let Some(packet) = heap.peek() {
        if packet.vtime <= now {
            // Check if guest can receive
            if unsafe {
                match (*(*state.client_ptr).info).can_receive {
                    Some(can_receive) => !can_receive(state.client_ptr),
                    None => false,
                }
            } {
                // Buffer to backlog
                let mut backlog =
                    state.backlog.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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

unsafe extern "C" fn zenoh_can_host_connect(ch: *mut CanHostState, _errp: *mut *mut Error) {
    let zch = ch as *mut ZenohCanHostState;

    if (*zch).node.is_null() || (*zch).topic.is_null() {
        return;
    }

    let topic_c = CStr::from_ptr((*zch).topic);
    let topic_str = topic_c.to_string_lossy().into_owned();

    let router_ptr = if (*zch).router.is_null() { ptr::null() } else { (*zch).router.cast_const() };

    let session = match open_session(router_ptr) {
        Ok(s) => s,
        Err(_) => return,
    };

    let publisher = session
        .declare_publisher(topic_str.clone())
        .wait()
        .unwrap_or_else(|_| std::process::abort());

    let (tx_rx, rx_rx) = unbounded::<Vec<u8>>();
    std::thread::spawn(move || {
        while let Ok(payload) = rx_rx.recv() {
            let _ = publisher.put(payload).wait();
        }
    });

    let (tx, rx) = unbounded();
    let earliest_vtime = Arc::new(AtomicU64::new(u64::MAX));

    // Prepare QEMU client struct
    (*zch).parent_obj.bus_client.info = &raw mut ZENOH_CAN_CLIENT_INFO;
    (*zch).parent_obj.bus_client.peer = zch as *mut CanBusClientState;

    let mut state = Box::new(State {
        session: session.clone(),
        subscriber: None, // Filled below to prevent partial move issues
        tx_sender: tx_rx,
        rx_sender: tx,
        rx_receiver: rx,
        local_heap: Mutex::new(BinaryHeap::new()),
        backlog: Mutex::new(VecDeque::new()),
        earliest_vtime: std::sync::Arc::clone(&earliest_vtime),
        rx_timer: None,
        client_ptr: &raw mut (*zch).parent_obj.bus_client,
    });

    let state_ptr = &raw mut *state;
    let rx_timer = Arc::new(unsafe {
        QomTimer::new(QEMU_CLOCK_VIRTUAL, rx_timer_cb, state_ptr as *mut core::ffi::c_void)
    });
    let rx_timer_clone = Arc::clone(&rx_timer);

    let tx_clone = state.rx_sender.clone();
    let subscriber = match SafeSubscriber::new(&session, &topic_str, move |sample| {
        let data = sample.payload().to_bytes();
        if let Ok(fbs) = root::<CanFdFrame>(&data) {
            let mut data_arr = [0u8; 64];
            let dlc = if let Some(d) = fbs.data() {
                let len = std::cmp::min(d.len(), 64);
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
            let packet = OrderedCanFrame { vtime, frame };

            if tx_clone.send(packet).is_ok() {
                // Update earliest vtime and wake BQL thread via timer_mod if it's sooner
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
    }) {
        Ok(s) => s,
        Err(_) => return,
    };

    state.subscriber = Some(subscriber);
    state.rx_timer = Some(rx_timer);
    (*zch).rust_state = Box::into_raw(state);

    can_bus_insert_client((*zch).parent_obj.bus, &raw mut (*zch).parent_obj.bus_client);
}

unsafe extern "C" fn zenoh_can_host_disconnect(ch: *mut CanHostState) {
    let zch = ch as *mut ZenohCanHostState;
    can_bus_remove_client(&raw mut (*zch).parent_obj.bus_client);

    if !(*zch).rust_state.is_null() {
        let mut state = Box::from_raw((*zch).rust_state);
        // Explicitly stop the subscriber first to wait for callbacks
        state.subscriber.take();
        state.rx_timer.take();
        (*zch).rust_state = ptr::null_mut();
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
    let zch = obj as *mut ZenohCanHostState;
    g_strdup((*zch).node)
}

unsafe extern "C" fn set_node(
    obj: *mut virtmcu_qom::qom::Object,
    value: *const c_char,
    _errp: *mut *mut Error,
) {
    let zch = obj as *mut ZenohCanHostState;
    if !(*zch).node.is_null() {
        g_free((*zch).node as *mut c_void);
    }
    (*zch).node = g_strdup(value);
}

unsafe extern "C" fn get_router(
    obj: *mut virtmcu_qom::qom::Object,
    _errp: *mut *mut Error,
) -> *mut c_char {
    let zch = obj as *mut ZenohCanHostState;
    g_strdup((*zch).router)
}

unsafe extern "C" fn set_router(
    obj: *mut virtmcu_qom::qom::Object,
    value: *const c_char,
    _errp: *mut *mut Error,
) {
    let zch = obj as *mut ZenohCanHostState;
    if !(*zch).router.is_null() {
        g_free((*zch).router as *mut c_void);
    }
    (*zch).router = g_strdup(value);
}

unsafe extern "C" fn get_topic(
    obj: *mut virtmcu_qom::qom::Object,
    _errp: *mut *mut Error,
) -> *mut c_char {
    let zch = obj as *mut ZenohCanHostState;
    g_strdup((*zch).topic)
}

unsafe extern "C" fn set_topic(
    obj: *mut virtmcu_qom::qom::Object,
    value: *const c_char,
    _errp: *mut *mut Error,
) {
    let zch = obj as *mut ZenohCanHostState;
    if !(*zch).topic.is_null() {
        g_free((*zch).topic as *mut c_void);
    }
    (*zch).topic = g_strdup(value);
}

unsafe extern "C" fn zenoh_can_host_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let chc = klass as *mut CanHostClass;
    (*chc).connect = Some(zenoh_can_host_connect);
    (*chc).disconnect = Some(zenoh_can_host_disconnect);

    object_class_property_add_str(klass, c"node".as_ptr(), Some(get_node), Some(set_node));
    object_class_property_add_str(klass, c"router".as_ptr(), Some(get_router), Some(set_router));
    object_class_property_add_str(klass, c"topic".as_ptr(), Some(get_topic), Some(set_topic));
}

unsafe extern "C" fn zenoh_can_host_instance_init(obj: *mut Object) {
    let zch = obj as *mut ZenohCanHostState;
    (*zch).node = ptr::null_mut();
    (*zch).router = ptr::null_mut();
    (*zch).topic = ptr::null_mut();
    (*zch).rust_state = ptr::null_mut();
}

unsafe extern "C" fn zenoh_can_host_instance_finalize(obj: *mut Object) {
    let zch = obj as *mut ZenohCanHostState;
    if !(*zch).node.is_null() {
        g_free((*zch).node as *mut c_void);
    }
    if !(*zch).router.is_null() {
        g_free((*zch).router as *mut c_void);
    }
    if !(*zch).topic.is_null() {
        g_free((*zch).topic as *mut c_void);
    }
    if !(*zch).rust_state.is_null() {
        let mut state = Box::from_raw((*zch).rust_state);
        // Explicitly drop the subscriber first
        state.subscriber.take();
        state.rx_timer.take();
    }
}

static ZENOH_CAN_HOST_TYPE_INFO: TypeInfo = TypeInfo {
    name: TYPE_CAN_HOST_ZENOH,
    parent: c"can-host".as_ptr(),
    instance_size: std::mem::size_of::<ZenohCanHostState>(),
    instance_align: std::mem::align_of::<ZenohCanHostState>(),
    instance_init: Some(zenoh_can_host_instance_init),
    instance_post_init: None,
    instance_finalize: Some(zenoh_can_host_instance_finalize),
    abstract_: false,
    class_size: std::mem::size_of::<CanHostClass>(),
    class_init: Some(zenoh_can_host_class_init),
    class_base_init: None,
    class_data: core::ptr::null(),
    interfaces: core::ptr::null(),
};

declare_device_type!(virtmcu_zenoh_canfd_init, ZENOH_CAN_HOST_TYPE_INFO);
