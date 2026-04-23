#![allow(unused_variables)]
#![allow(clippy::all)]

use core::ffi::{c_char, c_uint, c_void};
use crossbeam_channel::{bounded, Receiver, Sender};
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::ffi::CStr;
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};
use virtmcu_api::lin_generated::virtmcu::lin::{LinFrame, LinFrameArgs, LinMessageType};
use virtmcu_qom::irq::{qemu_irq, qemu_set_irq};
use virtmcu_qom::memory::{
    memory_region_init_io, MemoryRegion, MemoryRegionOps, DEVICE_LITTLE_ENDIAN,
};
use virtmcu_qom::qdev::{sysbus_init_irq, sysbus_init_mmio, SysBusDevice};
use virtmcu_qom::qom::{Object, ObjectClass, TypeInfo};
use virtmcu_qom::timer::{qemu_clock_get_ns, QomTimer, QEMU_CLOCK_VIRTUAL};
use virtmcu_qom::{
    declare_device_type, define_prop_string, define_prop_uint32, define_properties,
    device_class_set_props, error_setg,
};
use virtmcu_zenoh::SafeSubscriber;
use zenoh::pubsub::Publisher;
use zenoh::Session;
use zenoh::Wait;

const MAX_RX_FIFO: usize = 4;

#[repr(C)]
pub struct S32K144LpuartQemu {
    pub parent_obj: SysBusDevice,
    pub iomem: MemoryRegion,
    pub irq: qemu_irq,

    /* Properties */
    pub node_id: u32,
    pub router: *mut c_char,
    pub topic: *mut c_char,

    /* Rust state */
    pub rust_state: *mut LpuartState,
}

pub struct OrderedLinFrame {
    pub vtime: u64,
    pub msg_type: LinMessageType,
    pub data: Vec<u8>,
}

impl PartialEq for OrderedLinFrame {
    fn eq(&self, other: &Self) -> bool {
        self.vtime == other.vtime
    }
}
impl Eq for OrderedLinFrame {}
impl PartialOrd for OrderedLinFrame {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for OrderedLinFrame {
    fn cmp(&self, other: &Self) -> Ordering {
        other.vtime.cmp(&self.vtime) // Min-heap
    }
}

#[allow(dead_code)]
pub struct LpuartState {
    irq: qemu_irq,
    session: Session,
    publisher: Publisher<'static>,
    subscriber: Option<SafeSubscriber>,

    // Registers
    baud: u32,
    stat: u32,
    ctrl: u32,
    data: u32,
    match_: u32,
    modir: u32,
    fifo: u32,
    water: u32,

    // Internal state
    rx_buffer: Vec<u8>,

    // Deterministic delivery
    rx_sender: Sender<OrderedLinFrame>,
    rx_receiver: Receiver<OrderedLinFrame>,
    local_heap: Mutex<BinaryHeap<OrderedLinFrame>>,
    rx_timer: Option<Arc<QomTimer>>,
    earliest_vtime: Arc<AtomicU64>,
}

const REG_BAUD: u64 = 0x10;
const REG_STAT: u64 = 0x14;
const REG_CTRL: u64 = 0x18;
const REG_DATA: u64 = 0x1C;
const REG_MATCH: u64 = 0x20;
const REG_MODIR: u64 = 0x24;
const REG_FIFO: u64 = 0x28;
const REG_WATER: u64 = 0x2C;

const STAT_LBKDIF: u32 = 1 << 31;
const STAT_TDRE: u32 = 1 << 23;
const STAT_TC: u32 = 1 << 22;
const STAT_RDRF: u32 = 1 << 21;
const STAT_IDLE: u32 = 1 << 20;
const STAT_OR: u32 = 1 << 19;
const STAT_NF: u32 = 1 << 18;
const STAT_FE: u32 = 1 << 17;
const STAT_PF: u32 = 1 << 16;

const CTRL_TIE: u32 = 1 << 23;
const CTRL_TCIE: u32 = 1 << 22;
const CTRL_RIE: u32 = 1 << 21;
const CTRL_ILIE: u32 = 1 << 20;
const CTRL_TE: u32 = 1 << 19;
const CTRL_RE: u32 = 1 << 18;
const CTRL_SBK: u32 = 1 << 0;

const BAUD_LBKDIE: u32 = 1 << 31;
const BAUD_LBKDE: u32 = 1 << 24;

unsafe extern "C" fn lpuart_read(opaque: *mut c_void, offset: u64, size: c_uint) -> u64 {
    let s = &mut *(opaque as *mut S32K144LpuartQemu);
    if s.rust_state.is_null() {
        return 0;
    }
    let state = &mut *s.rust_state;
    match offset {
        0x00 => 0x04010001, // VERID
        0x04 => 0x00020202, // PARAM
        0x08 | 0x0C => 0,   // GLOBAL, PINCFG
        REG_BAUD => u64::from(state.baud),
        REG_STAT => u64::from(state.stat),
        REG_CTRL => u64::from(state.ctrl),
        REG_DATA => {
            let val = if state.rx_buffer.is_empty() {
                0
            } else {
                let byte = state.rx_buffer.remove(0);
                if state.rx_buffer.is_empty() {
                    state.stat &= !STAT_RDRF;
                }
                u32::from(byte)
            };
            u64::from(val)
        }
        REG_MATCH => u64::from(state.match_),
        REG_MODIR => u64::from(state.modir),
        REG_FIFO => u64::from(state.fifo),
        REG_WATER => u64::from(state.water),
        _ => 0,
    }
}

unsafe extern "C" fn lpuart_write(opaque: *mut c_void, offset: u64, value: u64, size: c_uint) {
    let s = &mut *(opaque as *mut S32K144LpuartQemu);
    if s.rust_state.is_null() {
        return;
    }
    let state = &mut *s.rust_state;
    let val = value as u32;

    match offset {
        REG_BAUD => state.baud = val,
        REG_STAT => {
            state.stat &=
                !(val & (STAT_LBKDIF | STAT_OR | STAT_NF | STAT_FE | STAT_PF | STAT_IDLE));
        }
        REG_CTRL => {
            let old_ctrl = state.ctrl;
            state.ctrl = val;
            if (state.ctrl & CTRL_SBK != 0) && (old_ctrl & CTRL_SBK == 0) {
                send_lin_msg(state, LinMessageType::Break, &[]);
            }
            update_irqs(state);
        }
        REG_DATA => {
            if state.ctrl & CTRL_TE != 0 {
                let byte = val as u8;
                send_lin_msg(state, LinMessageType::Data, &[byte]);
                state.stat |= STAT_TC | STAT_TDRE;
                update_irqs(state);
            }
        }
        REG_MATCH => state.match_ = val,
        REG_MODIR => state.modir = val,
        REG_FIFO => state.fifo = val,
        REG_WATER => state.water = val,
        _ => {}
    }
}

fn send_lin_msg(s: &mut LpuartState, msg_type: LinMessageType, data: &[u8]) {
    let mut fbb = flatbuffers::FlatBufferBuilder::new();
    let data_offset = fbb.create_vector(data);
    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;

    let args = LinFrameArgs { delivery_vtime_ns: now, type_: msg_type, data: Some(data_offset) };

    let frame = LinFrame::create(&mut fbb, &args);
    fbb.finish(frame, None);
    let finished_data = fbb.finished_data().to_vec();

    let _ = s.publisher.put(finished_data).wait();
}

fn update_irqs(s: &mut LpuartState) {
    let mut pending = false;
    if (s.ctrl & CTRL_TIE != 0) && (s.stat & STAT_TDRE != 0) {
        pending = true;
    }
    if (s.ctrl & CTRL_TCIE != 0) && (s.stat & STAT_TC != 0) {
        pending = true;
    }
    if (s.ctrl & CTRL_RIE != 0) && (s.stat & STAT_RDRF != 0) {
        pending = true;
    }
    if (s.ctrl & CTRL_ILIE != 0) && (s.stat & STAT_IDLE != 0) {
        pending = true;
    }
    if (s.baud & BAUD_LBKDIE != 0) && (s.stat & STAT_LBKDIF != 0) {
        pending = true;
    }

    unsafe {
        qemu_set_irq(s.irq, i32::from(pending));
    }
}

extern "C" fn lpuart_rx_timer_cb(opaque: *mut c_void) {
    let state = unsafe { &mut *(opaque as *mut LpuartState) };
    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;

    let mut next_vtime = None;

    {
        let mut heap = state.local_heap.lock().unwrap_or_else(std::sync::PoisonError::into_inner);

        while let Ok(packet) = state.rx_receiver.try_recv() {
            heap.push(packet);
        }

        while let Some(packet) = heap.peek() {
            if packet.vtime <= now {
                if let Some(packet) = heap.pop() {
                    match packet.msg_type {
                        LinMessageType::Break => {
                            if state.baud & BAUD_LBKDE != 0 {
                                state.stat |= STAT_LBKDIF;
                            }
                        }
                        LinMessageType::Data => {
                            if state.ctrl & CTRL_RE != 0 {
                                for byte in packet.data {
                                    if state.rx_buffer.len() >= MAX_RX_FIFO {
                                        state.stat |= STAT_OR;
                                    } else {
                                        state.rx_buffer.push(byte);
                                    }
                                }
                                if !state.rx_buffer.is_empty() {
                                    state.stat |= STAT_RDRF;
                                }
                            }
                        }
                        _ => {}
                    }
                }
            } else {
                next_vtime = Some(packet.vtime);
                break;
            }
        }
    }

    update_irqs(state);

    if let Some(vtime) = next_vtime {
        state.earliest_vtime.store(vtime, AtomicOrdering::Release);
        if let Some(rx_timer) = &state.rx_timer {
            rx_timer.mod_ns(vtime as i64);
        }
    } else {
        state.earliest_vtime.store(u64::MAX, AtomicOrdering::Release);
    }
}
static LPUART_OPS: MemoryRegionOps = MemoryRegionOps {
    read: Some(lpuart_read),
    write: Some(lpuart_write),
    read_with_attrs: ptr::null(),
    write_with_attrs: ptr::null(),
    endianness: DEVICE_LITTLE_ENDIAN,
    _padding1: [0; 4],
    valid: virtmcu_qom::memory::MemoryRegionValidRange {
        min_access_size: 1,
        max_access_size: 4,
        unaligned: false,
        _padding: [0; 7],
        accepts: ptr::null(),
    },
    impl_: virtmcu_qom::memory::MemoryRegionImplRange {
        min_access_size: 0,
        max_access_size: 0,
        unaligned: false,
        _padding: [0; 7],
    },
};

unsafe extern "C" fn lpuart_realize(dev: *mut c_void, errp: *mut *mut c_void) {
    let s = &mut *(dev as *mut S32K144LpuartQemu);

    let router_ptr = if s.router.is_null() { ptr::null() } else { s.router.cast_const() };

    let topic = if s.topic.is_null() {
        None
    } else {
        Some(unsafe { CStr::from_ptr(s.topic).to_string_lossy().into_owned() })
    };

    s.rust_state = lpuart_init_internal(s.irq, s.node_id, router_ptr, topic);
    if s.rust_state.is_null() {
        error_setg!(errp, "Failed to initialize Rust LPUART");
        return;
    }
}

unsafe extern "C" fn lpuart_instance_finalize(obj: *mut Object) {
    let s = &mut *(obj as *mut S32K144LpuartQemu);
    if !s.rust_state.is_null() {
        let mut state = Box::from_raw(s.rust_state);
        // Explicitly stop the subscriber first to wait for callbacks
        state.subscriber.take();
        state.rx_timer.take();
        s.rust_state = ptr::null_mut();
    }
}

unsafe extern "C" fn lpuart_instance_init(obj: *mut Object) {
    let s = &mut *(obj as *mut S32K144LpuartQemu);
    s.rust_state = ptr::null_mut();
    s.router = ptr::null_mut();
    s.topic = ptr::null_mut();

    memory_region_init_io(
        &raw mut s.iomem,
        obj,
        &raw const LPUART_OPS,
        obj as *mut c_void,
        c"s32k144-lpuart".as_ptr(),
        0x100,
    );
    sysbus_init_mmio(obj as *mut SysBusDevice, &raw mut s.iomem);
    sysbus_init_irq(obj as *mut SysBusDevice, &raw mut s.irq);
}

define_properties!(
    LPUART_PROPERTIES,
    [
        define_prop_uint32!(c"node".as_ptr(), S32K144LpuartQemu, node_id, 0),
        define_prop_string!(c"router".as_ptr(), S32K144LpuartQemu, router),
        define_prop_string!(c"topic".as_ptr(), S32K144LpuartQemu, topic),
    ]
);

unsafe extern "C" fn lpuart_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let dc = virtmcu_qom::device_class!(klass);
    unsafe {
        (*dc).realize = Some(lpuart_realize);
        (*dc).user_creatable = true;
    }
    device_class_set_props!(dc, LPUART_PROPERTIES);
}

static LPUART_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"s32k144-lpuart".as_ptr(),
    parent: c"sys-bus-device".as_ptr(),
    instance_size: std::mem::size_of::<S32K144LpuartQemu>(),
    instance_align: 0,
    instance_init: Some(lpuart_instance_init),
    instance_post_init: None,
    instance_finalize: Some(lpuart_instance_finalize),
    abstract_: false,
    class_size: 0,
    class_init: Some(lpuart_class_init),
    class_base_init: None,
    class_data: ptr::null(),
    interfaces: ptr::null(),
};

declare_device_type!(lpuart_type_init, LPUART_TYPE_INFO);

fn lpuart_init_internal(
    irq: qemu_irq,
    node_id: u32,
    router: *const c_char,
    topic: Option<String>,
) -> *mut LpuartState {
    let session = match unsafe { virtmcu_zenoh::open_session(router) } {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };

    let base_topic = topic.unwrap_or_else(|| "sim/lin".to_string());
    let tx_topic = format!("{base_topic}/{node_id}/tx");
    let rx_topic = format!("{base_topic}/{node_id}/rx");

    let publisher = match session.declare_publisher(tx_topic).wait() {
        Ok(p) => p,
        Err(_) => return ptr::null_mut(),
    };

    let (tx, rx) = bounded(1024);
    let earliest_vtime = Arc::new(AtomicU64::new(u64::MAX));
    let earliest_clone = Arc::clone(&earliest_vtime);

    let state_ptr_raw: *mut LpuartState =
        Box::into_raw(Box::<std::mem::MaybeUninit<LpuartState>>::new_uninit()).cast();

    let rx_timer = Arc::new(unsafe {
        QomTimer::new(QEMU_CLOCK_VIRTUAL, lpuart_rx_timer_cb, state_ptr_raw as *mut c_void)
    });
    let rx_timer_clone = Arc::clone(&rx_timer);

    let tx_clone = tx.clone();
    let subscriber = SafeSubscriber::new(&session, &rx_topic, move |sample| {
        let payload = sample.payload().to_bytes();
        let frame = match virtmcu_api::lin_generated::virtmcu::lin::root_as_lin_frame(&payload) {
            Ok(f) => f,
            Err(_) => return,
        };

        let vtime = frame.delivery_vtime_ns();
        let msg_type = frame.type_();
        let data = frame.data().map(|d| d.iter().collect()).unwrap_or_default();

        let packet = OrderedLinFrame { vtime, msg_type, data };

        if tx_clone.send(packet).is_ok() {
            let mut current = earliest_clone.load(AtomicOrdering::Relaxed);
            while vtime < current {
                if earliest_clone
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
                current = earliest_clone.load(AtomicOrdering::Relaxed);
            }
        }
    })
    .ok();

    let state = LpuartState {
        irq,
        session,
        publisher,
        subscriber,
        baud: 0x0F000004,
        stat: STAT_TDRE | STAT_TC,
        ctrl: 0,
        data: 0,
        match_: 0,
        modir: 0,
        fifo: 0,
        water: 0,
        rx_buffer: Vec::new(),
        rx_sender: tx,
        rx_receiver: rx,
        local_heap: Mutex::new(BinaryHeap::new()),
        rx_timer: Some(rx_timer),
        earliest_vtime,
    };

    unsafe { ptr::write(state_ptr_raw, state) };

    state_ptr_raw
}
