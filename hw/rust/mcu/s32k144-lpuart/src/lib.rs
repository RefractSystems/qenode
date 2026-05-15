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
//! S32K144 LPUART peripheral for VirtMCU simulation with pluggable transport.
use zenoh::Wait;

extern crate alloc;

use alloc::collections::VecDeque;
use alloc::sync::Arc;
use core::ffi::{c_char, c_uint, c_void};
use core::ptr;
use core::sync::atomic::AtomicU64;
use virtmcu_api::lin_generated::virtmcu::lin::{LinFrame, LinFrameArgs, LinMessageType};
use virtmcu_qom::irq::{qemu_set_irq, QemuIrq};
use virtmcu_qom::memory::{MemoryRegion, MemoryRegionOps, DEVICE_LITTLE_ENDIAN};
use virtmcu_qom::qdev::{sysbus_init_irq, sysbus_init_mmio, SysBusDevice};
use virtmcu_qom::qom::{ObjectClass, TypeInfo};
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

    /* Links */
    pub transport_hub: *mut virtmcu_qom::qom::Object,

    /* Rust state */
    /// Opaque pointer to the Rust backend state
    pub rust_state: *mut LpuartState,
}

const _: () = assert!(core::mem::offset_of!(S32K144LpuartQemu, parent_obj) == 0);
const _: () = assert!(core::mem::size_of::<S32K144LpuartQemu>() == 1152);

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct OrderedLinFrame {
    /// Virtual time of delivery
    pub vtime: u64,
    /// LIN message type
    pub msg_type: LinMessageType,
    /// Frame data
    pub data: Vec<u8>,
}

impl virtmcu_qom::sync::DeliveryPacket for OrderedLinFrame {
    fn delivery_vtime_ns(&self) -> u64 {
        self.vtime
    }
}

/// Internal state for LPUART
pub struct LpuartState {
    irq: QemuIrq,
    transport: Arc<dyn virtmcu_api::DataTransport>,
    receiver: Option<virtmcu_qom::sync::DeterministicReceiver<OrderedLinFrame>>,

    // Internal state
    inner: virtmcu_qom::sync::Mutex<LpuartInner>, // virtmcu-allow: mutex reasoning="State managed securely"
    tx_timer: Option<QomTimer>,

    tx_topic: String,
    pub _liveliness: Option<zenoh::liveliness::LivelinessToken>,
}

struct LpuartInner {
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
}

const REG_VERID: u64 = 0x00;
const REG_PARAM: u64 = 0x04;
const REG_GLOBAL: u64 = 0x08;
const REG_PINCFG: u64 = 0x0C;
const REG_BAUD: u64 = 0x10;
const REG_STAT: u64 = 0x14;
const REG_CTRL: u64 = 0x18;
const REG_DATA: u64 = 0x1C;
const REG_MATCH: u64 = 0x20;
const REG_MODIR: u64 = 0x24;
const REG_FIFO: u64 = 0x28;
const REG_WATER: u64 = 0x2C;

const LPUART_RESET_BAUD: u32 = 0x0F000004;
const LPUART_RESET_STAT: u32 = 0x00C00000;
const LPUART_RESET_FIFO: u32 = 0x00C00011;

const LPUART_DATA_MASK: u32 = 0xFF;
const LPUART_MAX_ACCESS_SIZE: u64 = 4;
const LPUART_MEM_REGION_SIZE: u64 = 0x100;

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

const LPUART_VERID: u64 = 0x04010001;
const LPUART_PARAM: u64 = 0x00020202;
const LPUART_TX_FIFO_CAP: usize = 4096;
const LPUART_SBR_MASK: u32 = 0x1FFF;
const LPUART_OSR_MASK: u32 = 0x1F;
const LPUART_OSR_SHIFT: u32 = 24;
const LPUART_DEFAULT_CLOCK_HZ: u32 = 48_000_000;
const LPUART_DEFAULT_BAUD_DELAY_NS: i64 = 86800;
const LPUART_BITS_PER_CHAR: i64 = 10;
const LPUART_NS_PER_SEC: i64 = 1_000_000_000;

/// # Safety
/// This function is called by QEMU on MMIO read. `opaque` must be a valid `S32K144LpuartQemu` pointer.
#[no_mangle]
pub unsafe extern "C" fn lpuart_read(opaque: *mut c_void, offset: u64, _size: c_uint) -> u64 {
    let s = unsafe { &mut *(opaque as *mut S32K144LpuartQemu) };
    if s.rust_state.is_null() {
        return 0;
    }
    let state = unsafe { &mut *s.rust_state };
    let mut inner = state.inner.lock();
    match offset {
        REG_VERID => LPUART_VERID,    // VERID
        REG_PARAM => LPUART_PARAM,    // PARAM
        REG_GLOBAL | REG_PINCFG => 0, // GLOBAL, PINCFG
        REG_BAUD => u64::from(inner.baud),
        REG_STAT => {
            // Note: In strict deterministic lock-free model, we cannot yield BQL here.
            // We return the state immediately. The caller should use polling correctly or interrupts.
            u64::from(inner.stat)
        }
        REG_CTRL => u64::from(inner.ctrl),
        REG_DATA => {
            let val = if inner.rx_buffer.is_empty() {
                0
            } else {
                let byte = inner.rx_buffer.remove(0);
                if inner.rx_buffer.is_empty() {
                    inner.stat &= !STAT_RDRF;
                }
                u32::from(byte)
            };
            u64::from(val)
        }
        REG_MATCH => u64::from(inner.match_),
        REG_MODIR => u64::from(inner.modir),
        REG_FIFO => u64::from(inner.fifo),
        REG_WATER => u64::from(inner.water),
        _ => {
            if s.debug {
                virtmcu_qom::sim_debug!("lpuart_read: unhandled offset 0x{:x}", offset);
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
    let mut inner = state.inner.lock();
    let val = value as u32;

    match offset {
        REG_BAUD => inner.baud = val,
        REG_STAT => {
            inner.stat &=
                !(val & (STAT_LBKDIF | STAT_OR | STAT_NF | STAT_FE | STAT_PF | STAT_IDLE));
        }
        REG_CTRL => {
            let old_ctrl = inner.ctrl;
            inner.ctrl = val;
            if (inner.ctrl & CTRL_SBK != 0) && (old_ctrl & CTRL_SBK == 0) {
                send_lin_msg(&*state.transport, &state.tx_topic, LinMessageType::Break, &[]);
            }
            update_irqs(state.irq, &inner);
        }
        REG_DATA if inner.ctrl & CTRL_TE != 0 => {
            let byte = u8::try_from(val & LPUART_DATA_MASK).expect("byte truncated");
            let was_empty = inner.tx_fifo.is_empty();
            if inner.tx_fifo.len() < LPUART_TX_FIFO_CAP {
                inner.tx_fifo.push_back(byte);
            }

            inner.stat &= !(STAT_TC | STAT_TDRE);
            update_irqs(state.irq, &inner);

            if was_empty {
                let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };
                if let Some(timer) = &state.tx_timer {
                    timer.mod_ns(now + calculate_baud_delay_ns(inner.baud));
                }
            }
        }
        REG_MATCH => inner.match_ = val,
        REG_MODIR => inner.modir = val,
        REG_FIFO => inner.fifo = val,
        REG_WATER => inner.water = val,
        _ => {
            if s.debug {
                virtmcu_qom::sim_debug!(
                    "lpuart_write: unhandled offset 0x{:x} val=0x{:x}",
                    offset,
                    value
                );
            }
        }
    }
}

fn send_lin_msg(
    transport: &dyn virtmcu_api::DataTransport,
    tx_topic: &str,
    msg_type: LinMessageType,
    data: &[u8],
) {
    virtmcu_qom::sim_info!("Sending LIN message to topic: {}", tx_topic);
    let mut fbb = flatbuffers::FlatBufferBuilder::new();
    let data_offset = fbb.create_vector(data);
    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;

    let args = LinFrameArgs { delivery_vtime_ns: now, type_: msg_type, data: Some(data_offset) };

    let frame = LinFrame::create(&mut fbb, &args);
    fbb.finish(frame, None);
    let finished_data = fbb.finished_data().to_vec();

    let _ = transport.publish(tx_topic, &finished_data);
}

fn update_irqs(irq: QemuIrq, inner: &LpuartInner) {
    let mut pending = false;
    if (inner.ctrl & CTRL_TIE != 0) && (inner.stat & STAT_TDRE != 0) {
        pending = true;
    }
    if (inner.ctrl & CTRL_TCIE != 0) && (inner.stat & STAT_TC != 0) {
        pending = true;
    }
    if (inner.ctrl & CTRL_RIE != 0) && (inner.stat & STAT_RDRF != 0) {
        pending = true;
    }
    if (inner.ctrl & CTRL_ILIE != 0) && (inner.stat & STAT_IDLE != 0) {
        pending = true;
    }
    if (inner.baud & BAUD_LBKDIE != 0) && (inner.stat & STAT_LBKDIF != 0) {
        pending = true;
    }

    unsafe {
        qemu_set_irq(irq, i32::from(pending));
    }
}

fn calculate_baud_delay_ns(baud_reg: u32) -> i64 {
    let sbr = baud_reg & LPUART_SBR_MASK;
    if sbr == 0 {
        return LPUART_DEFAULT_BAUD_DELAY_NS;
    }
    let osr = ((baud_reg >> LPUART_OSR_SHIFT) & LPUART_OSR_MASK) + 1;
    let baud_rate = LPUART_DEFAULT_CLOCK_HZ / (osr * sbr);
    if baud_rate == 0 {
        return LPUART_DEFAULT_BAUD_DELAY_NS;
    }
    (LPUART_NS_PER_SEC / i64::from(baud_rate)) * LPUART_BITS_PER_CHAR
}

extern "C" fn lpuart_tx_timer_cb(opaque: *mut c_void) {
    let state = unsafe { &mut *(opaque as *mut LpuartState) };
    let mut inner = state.inner.lock();

    if let Some(byte) = inner.tx_fifo.pop_front() {
        send_lin_msg(&*state.transport, &state.tx_topic, LinMessageType::Data, &[byte]);
    }

    if inner.tx_fifo.is_empty() {
        inner.stat |= STAT_TC | STAT_TDRE;
        update_irqs(state.irq, &inner);
    } else {
        let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };
        if let Some(timer) = &state.tx_timer {
            timer.mod_ns(now + calculate_baud_delay_ns(inner.baud));
        }
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
        max_access_size: LPUART_MAX_ACCESS_SIZE as u32,
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
        let mut len = 0;
        while len < 1024 {
            if unsafe { *s.router.add(len) } == 0 {
                break;
            }
            len += 1;
        }
        let slice = unsafe { core::slice::from_raw_parts(s.router.cast::<u8>(), len) };
        String::from_utf8_lossy(slice).into_owned()
    };
    virtmcu_qom::sim_info!("ROUTER STRING: {}", router_addr);
    let transport_name = if !s.transport.is_null() {
        let mut len = 0;
        while len < 1024 {
            if unsafe { *s.transport.add(len) } == 0 {
                break;
            }
            len += 1;
        }
        let slice = unsafe { core::slice::from_raw_parts(s.transport.cast::<u8>(), len) };
        String::from_utf8_lossy(slice).into_owned()
    } else if std::path::Path::new(&router_addr)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("sock"))
        || router_addr.starts_with("/tmp/") // virtmcu-allow: absolute_path reasoning="Legacy script"
        || router_addr.starts_with("unix:")
    {
        "unix".to_owned()
    } else {
        "zenoh".to_owned()
    };

    let topic = if s.topic.is_null() {
        None
    } else {
        let mut len = 0;
        while len < 1024 {
            if unsafe { *s.topic.add(len) } == 0 {
                break;
            }
            len += 1;
        }
        let slice = unsafe { core::slice::from_raw_parts(s.topic.cast::<u8>(), len) };
        Some(String::from_utf8_lossy(slice).into_owned())
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
        state.receiver.take();
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
            LPUART_MEM_REGION_SIZE,
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
    let mut inner = state.inner.lock();

    inner.baud = LPUART_RESET_BAUD;
    inner.stat = LPUART_RESET_STAT;
    inner.ctrl = 0;
    inner.match_ = 0;
    inner.modir = 0;
    inner.fifo = LPUART_RESET_FIFO;
    inner.water = 0;

    inner.rx_buffer.clear();
    inner.tx_fifo.clear();

    if let Some(timer) = &state.tx_timer {
        timer.del();
    }
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
        match transport_unix::UdsDataTransport::new(&path) {
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

fn decode_lpuart(_opaque: *mut c_void, _topic: &str, data: &[u8]) -> Option<OrderedLinFrame> {
    virtmcu_qom::sim_info!("Zenoh subscriber received packet of len {}", data.len());
    let frame = virtmcu_api::lin_generated::virtmcu::lin::root_as_lin_frame(data).ok()?;

    let vtime = frame.delivery_vtime_ns();
    let msg_type = frame.type_();
    let data = frame.data().map(|d| d.iter().collect()).unwrap_or_default();

    Some(OrderedLinFrame { vtime, msg_type, data })
}

fn deliver_lpuart(opaque: *mut c_void, packet: OrderedLinFrame) {
    let state = unsafe { &mut *(opaque as *mut LpuartState) };
    let mut inner = state.inner.lock();

    match packet.msg_type {
        LinMessageType::Sync => {
            inner.rx_buffer.clear();
            inner.rx_buffer.extend_from_slice(&packet.data);
            inner.stat |= STAT_RDRF;
        }
        LinMessageType::Break if inner.baud & BAUD_LBKDE != 0 => {
            inner.stat |= STAT_LBKDIF;
        }
        LinMessageType::Data if inner.ctrl & CTRL_RE != 0 => {
            for byte in packet.data {
                if inner.rx_buffer.len() >= MAX_RX_FIFO {
                    inner.stat |= STAT_OR;
                } else {
                    inner.rx_buffer.push(byte);
                }
            }
            if !inner.rx_buffer.is_empty() {
                inner.stat |= STAT_RDRF;
            }
        }
        _ => {}
    }

    let irq = state.irq;
    update_irqs(irq, &inner);
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

    let mut state_box = Box::new(LpuartState {
        _liveliness: liveliness,
        irq,
        transport: Arc::clone(&transport),
        receiver: None,
        inner: virtmcu_qom::sync::Mutex::new(LpuartInner {
            baud: LPUART_RESET_BAUD,
            stat: STAT_TDRE | STAT_TC,
            ctrl: 0,
            _data: 0,
            match_: 0,
            modir: 0,
            fifo: 0,
            water: 0,
            rx_buffer: Vec::new(),
            tx_fifo: VecDeque::new(),
        }), // virtmcu-allow: mutex reasoning="State managed securely"
        tx_timer: None,
        tx_topic,
    });

    let state_ptr = core::ptr::from_mut(&mut *state_box);

    let generation = Arc::new(AtomicU64::new(0));

    match virtmcu_qom::sync::DeterministicReceiver::new(
        &*transport,
        &rx_topic,
        generation,
        state_ptr as *mut c_void,
        decode_lpuart,
        deliver_lpuart,
    ) {
        Ok(receiver) => {
            state_box.receiver = Some(receiver);
            virtmcu_qom::sim_info!("SUCCESSFULLY CREATED SUBSCRIPTION to {}", rx_topic);
        }
        Err(e) => {
            virtmcu_qom::sim_err!("FAILED TO CREATE SUBSCRIPTION!: {}", e);
            return ptr::null_mut();
        }
    }

    state_box.tx_timer = Some(unsafe {
        QomTimer::new(QEMU_CLOCK_VIRTUAL, lpuart_tx_timer_cb, state_ptr as *mut c_void)
    });

    Box::into_raw(state_box)
}

#[cfg(test)]
#[allow(clippy::magic_numbers)] // virtmcu-allow: allow reasoning="Legacy test module exceptions"
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
