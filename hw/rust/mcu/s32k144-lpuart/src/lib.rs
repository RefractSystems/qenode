//! S32K144 LPUART peripheral for VirtMCU simulation with pluggable transport.
use zenoh::Wait;

extern crate alloc;

use alloc::collections::{BinaryHeap, VecDeque};
use alloc::sync::Arc;
use core::cmp::Ordering;
use core::ffi::{c_char, c_uint, c_void, CStr};
use core::ptr;
use core::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use crossbeam_channel::{bounded, Receiver, Sender};
use virtmcu_api::lin_generated::virtmcu::lin::{LinFrame, LinFrameArgs, LinMessageType};
use virtmcu_qom::irq::{qemu_set_irq, QemuIrq};
use virtmcu_qom::memory::{MemoryRegion, MemoryRegionOps, DEVICE_LITTLE_ENDIAN};
use virtmcu_qom::qdev::{sysbus_init_irq, sysbus_init_mmio, SysBusDevice};
use virtmcu_qom::qom::{ObjectClass, TypeInfo};
use virtmcu_qom::sync::{BqlGuarded, SafeSubscription};
use virtmcu_qom::timer::{qemu_clock_get_ns, QomTimer, QEMU_CLOCK_VIRTUAL};
use virtmcu_qom::{
    declare_device_type, define_prop_string, define_prop_uint32, define_properties,
    device_class_set_props, error_setg,
};

const MAX_RX_FIFO: usize = 4;

/// S32K144 LPUART QEMU object structure
#[repr(C)]
pub struct S32K144LpuartQemu {
    /// Parent object
    pub parent_obj: SysBusDevice,
    /// I/O memory region
    pub iomem: MemoryRegion,
    /// IRQ line
    pub irq: QemuIrq,

    /* Properties */
    /// Unique node ID
    pub node_id: u32,
    /// The transport to use (zenoh or unix)
    pub transport: *mut c_char,
    /// Optional router address
    pub router: *mut c_char,
    /// Optional base topic
    pub topic: *mut c_char,
    /// Enable debug logging
    pub debug: bool,

    /* Rust state */
    /// Opaque pointer to the Rust backend state
    pub rust_state: *mut LpuartState,
}

/// Ordered LIN frame for deterministic delivery
pub struct OrderedLinFrame {
    /// Virtual time of delivery
    pub vtime: u64,
    /// LIN message type
    pub msg_type: LinMessageType,
    /// Frame data
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

/// Internal state for LPUART
pub struct LpuartState {
    irq: QemuIrq,
    transport: Arc<dyn virtmcu_api::DataTransport>,
    subscription: Option<SafeSubscription>,

    // Registers
    baud: u32,
    stat: u32,
    ctrl: u32,
    _data: u32,
    match_: u32,
    modir: u32,
    fifo: u32,
    water: u32,

    // Internal state
    rx_buffer: Vec<u8>,
    tx_fifo: VecDeque<u8>,
    tx_timer: Option<QomTimer>,

    // Deterministic delivery
    _rx_sender: Sender<OrderedLinFrame>,
    rx_receiver: Receiver<OrderedLinFrame>,
    local_heap: BqlGuarded<BinaryHeap<OrderedLinFrame>>,
    rx_timer: Option<Arc<QomTimer>>,
    earliest_vtime: Arc<AtomicU64>,
    tx_topic: String,
    pub _liveliness: Option<zenoh::liveliness::LivelinessToken>,
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

/// # Safety
/// This function is called by QEMU on MMIO read. `opaque` must be a valid `S32K144LpuartQemu` pointer.
#[no_mangle]
pub unsafe extern "C" fn lpuart_read(opaque: *mut c_void, offset: u64, _size: c_uint) -> u64 {
    let s = unsafe { &mut *(opaque as *mut S32K144LpuartQemu) };
    if s.rust_state.is_null() {
        return 0;
    }
    let state = unsafe { &mut *s.rust_state };
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
        _ => {
            if s.debug {
                virtmcu_qom::sim_warn!("lpuart_read: unhandled offset 0x{:x}", offset);
            }
            0
        }
    }
}

/// # Safety
/// This function is called by QEMU on MMIO write. `opaque` must be a valid `S32K144LpuartQemu` pointer.
#[no_mangle]
pub unsafe extern "C" fn lpuart_write(opaque: *mut c_void, offset: u64, value: u64, _size: c_uint) {
    let s = unsafe { &mut *(opaque as *mut S32K144LpuartQemu) };
    if s.rust_state.is_null() {
        return;
    }
    let state = unsafe { &mut *s.rust_state };
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
        REG_DATA if state.ctrl & CTRL_TE != 0 => {
            let byte = val as u8;
            let was_empty = state.tx_fifo.is_empty();
            if state.tx_fifo.len() < 4096 {
                state.tx_fifo.push_back(byte);
            }

            state.stat &= !(STAT_TC | STAT_TDRE);
            update_irqs(state);

            if was_empty {
                let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };
                if let Some(timer) = &state.tx_timer {
                    timer.mod_ns(now + calculate_baud_delay_ns(state.baud));
                }
            }
        }
        REG_MATCH => state.match_ = val,
        REG_MODIR => state.modir = val,
        REG_FIFO => state.fifo = val,
        REG_WATER => state.water = val,
        _ => {
            if s.debug {
                virtmcu_qom::sim_warn!(
                    "lpuart_write: unhandled offset 0x{:x} val=0x{:x}",
                    offset,
                    value
                );
            }
        }
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

    let _ = s.transport.publish(&s.tx_topic, &finished_data);
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

fn calculate_baud_delay_ns(baud_reg: u32) -> i64 {
    let sbr = baud_reg & 0x1FFF;
    if sbr == 0 {
        return 86800;
    }
    let osr = ((baud_reg >> 24) & 0x1F) + 1;
    let baud_rate = 48_000_000 / (osr * sbr);
    if baud_rate == 0 {
        return 86800;
    }
    ((1_000_000_000 / baud_rate) * 10) as i64
}

extern "C" fn lpuart_tx_timer_cb(opaque: *mut c_void) {
    let state = unsafe { &mut *(opaque as *mut LpuartState) };

    if let Some(byte) = state.tx_fifo.pop_front() {
        send_lin_msg(state, LinMessageType::Data, &[byte]);
    }

    if state.tx_fifo.is_empty() {
        state.stat |= STAT_TC | STAT_TDRE;
        update_irqs(state);
    } else {
        let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };
        if let Some(timer) = &state.tx_timer {
            timer.mod_ns(now + calculate_baud_delay_ns(state.baud));
        }
    }
}

extern "C" fn lpuart_rx_timer_cb(opaque: *mut c_void) {
    let state = unsafe { &mut *(opaque as *mut LpuartState) };
    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;

    let mut next_vtime = None;

    {
        let mut heap = state.local_heap.get_mut();

        while let Ok(packet) = state.rx_receiver.try_recv() {
            heap.push(packet);
        }

        while let Some(packet) = heap.peek() {
            if packet.vtime <= now {
                if let Some(p) = heap.pop() {
                    match p.msg_type {
                        LinMessageType::Break if state.baud & BAUD_LBKDE != 0 => {
                            state.stat |= STAT_LBKDIF;
                        }
                        LinMessageType::Data if state.ctrl & CTRL_RE != 0 => {
                            for byte in p.data {
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

/// # Safety
/// This function is called by QEMU to realize the device. `dev` must be a valid `S32K144LpuartQemu` pointer.
#[no_mangle]
pub unsafe extern "C" fn lpuart_realize(dev: *mut c_void, errp: *mut *mut c_void) {
    let s = unsafe { &mut *(dev as *mut S32K144LpuartQemu) };

    let router_ptr = if s.router.is_null() { ptr::null() } else { s.router.cast_const() };
    virtmcu_qom::sim_info!(
        "ROUTER: {:?}, TRANSPORT: {:?}, TOPIC: {:?}",
        s.router,
        s.transport,
        s.topic
    );
    let router_addr = if s.router.is_null() {
        String::new()
    } else {
        unsafe { core::ffi::CStr::from_ptr(s.router).to_string_lossy().into_owned() }
    };
    let transport_name = if !s.transport.is_null() {
        unsafe { core::ffi::CStr::from_ptr(s.transport).to_string_lossy().into_owned() }
    } else if std::path::Path::new(&router_addr)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("sock"))
        || router_addr.starts_with("/tmp/")
        || router_addr.starts_with("unix:")
    {
        "unix".to_owned()
    } else {
        "zenoh".to_owned()
    };

    let topic = if s.topic.is_null() {
        None
    } else {
        Some(unsafe { CStr::from_ptr(s.topic).to_string_lossy().into_owned() })
    };

    s.rust_state = lpuart_init_internal(s.irq, s.node_id, transport_name, router_ptr, topic);
    if s.rust_state.is_null() {
        error_setg!(errp, "Failed to initialize Rust LPUART");
    }
}

/// # Safety
/// This function is called by QEMU when finalizing the device. `obj` must be a valid `S32K144LpuartQemu` pointer.
#[no_mangle]
pub unsafe extern "C" fn lpuart_instance_finalize(obj: *mut virtmcu_qom::qom::Object) {
    let s = unsafe { &mut *(obj as *mut S32K144LpuartQemu) };
    if !s.rust_state.is_null() {
        let mut state = unsafe { Box::from_raw(s.rust_state) };
        state.subscription.take();
        state.rx_timer.take();
        state.tx_timer.take();
        s.rust_state = ptr::null_mut();
    }
}

/// # Safety
/// This function is called by QEMU on object initialization. `obj` must be a valid `S32K144LpuartQemu` pointer.
#[no_mangle]
pub unsafe extern "C" fn lpuart_instance_init(obj: *mut virtmcu_qom::qom::Object) {
    let s = unsafe { &mut *(obj as *mut S32K144LpuartQemu) };
    s.rust_state = ptr::null_mut();
    s.transport = ptr::null_mut();
    s.router = ptr::null_mut();
    s.topic = ptr::null_mut();

    unsafe {
        virtmcu_qom::memory::memory_region_init_io(
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
}

define_properties!(
    LPUART_PROPERTIES,
    [
        define_prop_uint32!(c"node".as_ptr(), S32K144LpuartQemu, node_id, 0),
        define_prop_string!(c"transport".as_ptr(), S32K144LpuartQemu, transport),
        define_prop_string!(c"router".as_ptr(), S32K144LpuartQemu, router),
        define_prop_string!(c"topic".as_ptr(), S32K144LpuartQemu, topic),
        virtmcu_qom::define_prop_bool!(c"debug".as_ptr(), S32K144LpuartQemu, debug, false),
    ]
);

/// # Safety
/// This function is called by QEMU to initialize the class. `klass` must be a valid `ObjectClass` pointer.
unsafe extern "C" fn lpuart_reset(dev: *mut c_void) {
    let s = unsafe { &mut *(dev as *mut S32K144LpuartQemu) };
    if s.rust_state.is_null() {
        return;
    }
    let state = unsafe { &mut *s.rust_state };

    state.baud = 0x0F000004;
    state.stat = 0xC0000000;
    state.ctrl = 0;
    state.match_ = 0;
    state.modir = 0;
    state.fifo = 0x00C00011;
    state.water = 0;

    state.rx_buffer.clear();
    state.tx_fifo.clear();

    if let Some(timer) = &state.rx_timer {
        timer.del();
    }
    if let Some(timer) = &state.tx_timer {
        timer.del();
    }

    state.earliest_vtime.store(u64::MAX, AtomicOrdering::Release);
}

pub unsafe extern "C" fn lpuart_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let dc = virtmcu_qom::device_class!(klass);
    unsafe {
        (*dc).realize = Some(lpuart_realize);
        (*dc).legacy_reset = Some(lpuart_reset);
        (*dc).user_creatable = true;
    }
    device_class_set_props!(dc, LPUART_PROPERTIES);
}

#[used]
static LPUART_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"s32k144-lpuart".as_ptr(),
    parent: c"sys-bus-device".as_ptr(),
    instance_size: core::mem::size_of::<S32K144LpuartQemu>(),
    instance_align: 0,
    instance_init: Some(lpuart_instance_init),
    instance_post_init: None,
    instance_finalize: Some(lpuart_instance_finalize),
    abstract_: false,
    class_size: core::mem::size_of::<virtmcu_qom::qdev::SysBusDeviceClass>(),
    class_init: Some(lpuart_class_init),
    class_base_init: None,
    class_data: ptr::null(),
    interfaces: ptr::null(),
};

declare_device_type!(LPUART_TYPE_INIT, LPUART_TYPE_INFO);

fn create_transport(
    transport_name: &str,
    router: *const c_char,
) -> Option<Arc<dyn virtmcu_api::DataTransport>> {
    if transport_name == "unix" {
        let path = unsafe { core::ffi::CStr::from_ptr(router).to_string_lossy().into_owned() };
        virtmcu_qom::sim_info!("LPUART path = {}", path);
        match transport_unix::UnixDataTransport::new(&path) {
            Ok(t) => Some(Arc::new(t)),
            Err(e) => {
                virtmcu_qom::sim_err!("UNIX DATA TRANSPORT ERROR: {}", e);
                None
            }
        }
    } else {
        match unsafe { transport_zenoh::get_or_init_session(router) } {
            Ok(s) => Some(Arc::new(transport_zenoh::ZenohDataTransport::new(s))),
            Err(e) => {
                virtmcu_qom::sim_err!("UNIX DATA TRANSPORT ERROR: {}", e);
                None
            }
        }
    }
}

fn create_subscription(
    transport: &Arc<dyn virtmcu_api::DataTransport>,
    rx_topic: &str,
    rx_timer: &Arc<QomTimer>,
    earliest_vtime: &Arc<AtomicU64>,
    tx_sender: Sender<OrderedLinFrame>,
) -> Option<SafeSubscription> {
    let rx_timer_clone = Arc::clone(rx_timer);
    let earliest_clone = Arc::clone(earliest_vtime);

    let sub_callback: virtmcu_api::DataCallback = Box::new(move |data| {
        let frame = match virtmcu_api::lin_generated::virtmcu::lin::root_as_lin_frame(data) {
            Ok(f) => f,
            Err(e) => {
                virtmcu_qom::sim_warn!("Failed to parse LinFrame: {:?}", e);
                return;
            }
        };

        let vtime = frame.delivery_vtime_ns();
        let msg_type = frame.type_();
        let data = frame.data().map(|d| d.iter().collect()).unwrap_or_default();

        let packet = OrderedLinFrame { vtime, msg_type, data };

        if tx_sender.send(packet).is_ok() {
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
    });

    let generation = Arc::new(AtomicU64::new(0));
    SafeSubscription::new(&**transport, rx_topic, generation, sub_callback).ok()
}

fn lpuart_init_internal(
    irq: QemuIrq,
    node_id: u32,
    transport_name: String,
    router: *const c_char,
    topic: Option<String>,
) -> *mut LpuartState {
    virtmcu_qom::sim_info!("TRANSPORT NAME IS: {:?}", transport_name);
    let transport = match create_transport(&transport_name, router) {
        Some(t) => t,
        None => return ptr::null_mut(),
    };

    let base_topic = topic.unwrap_or_else(|| "sim/lin".to_owned());
    let tx_topic = format!("{base_topic}/{node_id}/tx");
    let rx_topic = format!("{base_topic}/{node_id}/rx");

    let (tx, rx) = bounded(1024);
    let earliest_vtime = Arc::new(AtomicU64::new(u64::MAX));

    let state_ptr_raw: *mut LpuartState =
        Box::into_raw(Box::<core::mem::MaybeUninit<LpuartState>>::new_uninit()).cast();

    let rx_timer = Arc::new(unsafe {
        QomTimer::new(QEMU_CLOCK_VIRTUAL, lpuart_rx_timer_cb, state_ptr_raw as *mut c_void)
    });

    let tx_timer = unsafe {
        QomTimer::new(QEMU_CLOCK_VIRTUAL, lpuart_tx_timer_cb, state_ptr_raw as *mut c_void)
    };

    let subscription =
        create_subscription(&transport, &rx_topic, &rx_timer, &earliest_vtime, tx.clone());

    let liveliness = if transport_name == "zenoh" {
        match unsafe { transport_zenoh::get_or_init_session(router) } {
            Ok(session) => {
                let hb_topic = format!("sim/s32k144-lpuart/liveliness/{node_id}");
                session.liveliness().declare_token(hb_topic).wait().ok()
            }
            Err(_) => None,
        }
    } else {
        None
    };
    let state = LpuartState {
        _liveliness: liveliness,
        irq,
        transport,
        subscription,
        baud: 0x0F000004,
        stat: STAT_TDRE | STAT_TC,
        ctrl: 0,
        _data: 0,
        match_: 0,
        modir: 0,
        fifo: 0,
        water: 0,
        rx_buffer: Vec::new(),
        tx_fifo: VecDeque::new(),
        tx_timer: Some(tx_timer),
        _rx_sender: tx,
        rx_receiver: rx,
        local_heap: BqlGuarded::new(BinaryHeap::new()),
        rx_timer: Some(rx_timer),
        earliest_vtime,
        tx_topic,
    };

    unsafe { ptr::write(state_ptr_raw, state) };

    state_ptr_raw
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_s32k144_lpuart_qemu_layout() {
        assert_eq!(
            core::mem::offset_of!(S32K144LpuartQemu, parent_obj),
            0,
            "SysBusDevice must be the first field"
        );
    }
}
