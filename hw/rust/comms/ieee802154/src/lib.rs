#![cfg_attr(
    test,
    allow(
        clippy::expect_used,
        clippy::unwrap_used,
        clippy::panic,
        clippy::indexing_slicing,
        clippy::panic_in_result_fn
    )
)]
// std is required: zenoh/tokio bring std
//! Virtmcu 802.15.4 radio with pluggable transport.
use zenoh::Wait;

extern crate alloc;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ffi::{c_char, c_uint, c_void, CStr};
use core::ptr;
use virtmcu_api::rf802154;
use virtmcu_api::Rf802154Mhr;
use virtmcu_qom::irq::{qemu_set_irq, QemuIrq};
use virtmcu_qom::memory::{
    memory_region_init_io, MemoryRegion, MemoryRegionOps, DEVICE_LITTLE_ENDIAN,
};
use virtmcu_qom::qdev::{sysbus_init_irq, sysbus_init_mmio, SysBusDevice};
use virtmcu_qom::qom::{Object, ObjectClass, TypeInfo};
use virtmcu_qom::sync::{BqlGuarded, SafeSubscription}; // virtmcu-allow: bql reasoning="Safe Zenoh integration"
use virtmcu_qom::timer::{qemu_clock_get_ns, QomTimer, QEMU_CLOCK_VIRTUAL};
use virtmcu_qom::{
    declare_device_type, define_prop_string, define_prop_uint32, define_properties, device_class,
    error_setg,
};

use core::cmp::Ordering;

const IEEE_MMIO_SIZE: u64 = 0x100;
const IEEE_MAX_ACCESS: u32 = 8;
const IEEE_FIFO_SIZE: usize = 128;
const IEEE_RX_QUEUE_SIZE: usize = 16;
const IEEE_DEFAULT_BE: u8 = 3;

// Register Offsets
const REG_TX_DATA: u64 = 0x00;
const REG_TX_LEN: u64 = 0x04;
const REG_TX_GO: u64 = 0x08;
const REG_RX_DATA: u64 = 0x0C;
const REG_RX_LEN: u64 = 0x10;
const REG_STATUS: u64 = 0x14;
const REG_RSSI: u64 = 0x18;
const REG_STATE: u64 = 0x1C;
const REG_PAN_ID: u64 = 0x20;
const REG_SHORT_ADDR: u64 = 0x24;
const REG_EXT_ADDR_LO: u64 = 0x28;
const REG_EXT_ADDR_HI: u64 = 0x2C;

// Status bits
const STATUS_RX_PENDING: u32 = 0x01;
const STATUS_TX_DONE: u32 = 0x02;

// Masks and Shifts
const TX_LEN_MASK: u64 = 0x7F;
const STATE_SHIFT: u32 = 8;
const ADDR_32_MASK: u64 = 0xFFFFFFFF;
const ADDR_32_SHIFT: u32 = 32;

// Addressing
const IEEE_BROADCAST_PAN: u16 = 0xFFFF;
const IEEE_BROADCAST_ADDR: u16 = 0xFFFF;
const IEEE_ADDR_MODE_NONE: u16 = 0x00;
const IEEE_ADDR_MODE_SHORT: u16 = 0x02;
const IEEE_ADDR_MODE_EXT: u16 = 0x03;
const IEEE_ADDR_MODE_SHIFT: u32 = 10;
const IEEE_ADDR_MODE_MASK: u16 = 0x03;

// Frame and Timing
const IEEE_NS_PER_BYTE: u64 = 32_000;
const IEEE_OVERHEAD_BYTES: u64 = 6;
const IEEE_ACK_REQUEST_BIT: u16 = 1 << 5;
const IEEE_ACK_FRAME_TYPE: u8 = 0x02;
const IEEE_DEFAULT_LQI: u8 = 255;

// FNV-1a constants for deterministic random
const FNV_OFFSET_BASIS: u32 = 0x811c9dc5;
const FNV_PRIME: u32 = 0x01000193;

// Backoff and Timing
const MAC_MIN_BE: u8 = 3;
const UNIT_BACKOFF_PERIOD_NS: u64 = 320_000; // 20 symbols * 16 us/symbol
const SIFS_NS: u64 = 192_000; // 12 symbols * 16 us/symbol

// Byte slicing for deterministic_random
const NODE_ID_OFFSET: usize = 0;
const NODE_ID_SIZE: usize = 4;
const VTIME_OFFSET: usize = 4;
const VTIME_SIZE: usize = 8;
const EXTRA_OFFSET: usize = 12;
const EXTRA_SIZE: usize = 8;
const HASH_BYTES_LEN: usize = 20;

// ACK frame
const ACK_RESERVED_BYTE: u8 = 0x00;

// Radio States
const RADIO_STATE_OFF: u64 = 0;
const RADIO_STATE_IDLE: u64 = 1;
const RADIO_STATE_RX: u64 = 2;
const RADIO_STATE_TX: u64 = 3;

#[repr(C)]
pub struct Virtmcu802154QEMU {
    pub parent_obj: SysBusDevice,
    pub iomem: MemoryRegion,
    pub irq: QemuIrq,

    /* Properties */
    pub node_id: u32,
    pub transport: *mut c_char,
    pub router: *mut c_char,
    pub topic: *mut c_char,
    pub debug: bool,

    /* Rust state */
    pub rust_state: *mut Virtmcu802154State,
}

struct RxFrame {
    delivery_vtime: u64,
    sequence: u64,
    data: [u8; 128],
    size: usize,
    rssi: i8,
}

#[repr(u8)]
#[derive(Copy, Clone, PartialEq, Eq)]
enum RadioState {
    Off = 0,
    Idle = 1,
    Rx = 2,
    Tx = 3,
}

pub struct Virtmcu802154State {
    parent_ptr: *mut Virtmcu802154QEMU,
    irq: QemuIrq,
    transport: Arc<dyn virtmcu_api::DataTransport>,
    topic_tx: String,
    subscription: Option<SafeSubscription>, // virtmcu-allow: bql reasoning="SafeSubscription ensures thread safety for Zenoh callbacks"
    rx_timer: Option<QomTimer>,
    backoff_timer: Option<QomTimer>,
    ack_timer: Option<QomTimer>,
    tx_timer: Option<QomTimer>,

    // All state accessed exclusively under BQL; see BqlGuarded docs.
    inner: BqlGuarded<Virtmcu802154Inner>,
    pub _liveliness: Option<zenoh::liveliness::LivelinessToken>,
}

struct Virtmcu802154Inner {
    node_id: u32,
    tx_fifo: [u8; 128],
    tx_len: u32,
    rx_fifo: [u8; 128],
    rx_len: u32,
    rx_read_pos: u32,
    rx_rssi: i8,
    status: u32,
    state: RadioState,

    pan_id: u16,
    short_addr: u16,
    ext_addr: u64,

    rx_queue: Vec<RxFrame>,

    // CSMA/CA state
    nb: u8,
    be: u8,

    // Auto-ACK state
    ack_pending: bool,
    ack_seq: u8,
    tx_sequence: u64,
}

extern "C" fn ieee802154_read(opaque: *mut c_void, offset: u64, _size: c_uint) -> u64 {
    let s = unsafe { &mut *(opaque as *mut Virtmcu802154QEMU) };
    if s.rust_state.is_null() {
        return 0;
    }
    let rust_state = unsafe { &mut *s.rust_state };
    ieee802154_read_internal(rust_state, offset)
}

extern "C" fn ieee802154_write(opaque: *mut c_void, offset: u64, value: u64, _size: c_uint) {
    let s = unsafe { &mut *(opaque as *mut Virtmcu802154QEMU) };
    if s.rust_state.is_null() {
        return;
    }
    let rust_state = unsafe { &mut *s.rust_state };
    ieee802154_write_internal(rust_state, offset, value);
}

static VIRTM_802154_OPS: MemoryRegionOps = MemoryRegionOps {
    read: Some(ieee802154_read),
    write: Some(ieee802154_write),
    read_with_attrs: ptr::null(),
    write_with_attrs: ptr::null(),
    endianness: DEVICE_LITTLE_ENDIAN,
    _padding1: [0; 4],
    valid: virtmcu_qom::memory::MemoryRegionValidRange {
        min_access_size: 1,
        max_access_size: IEEE_MAX_ACCESS,
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

extern "C" fn ieee802154_realize(dev: *mut c_void, errp: *mut *mut c_void) {
    let s = unsafe { &mut *(dev as *mut Virtmcu802154QEMU) };

    let node = s.node_id.to_string();
    let transport_name = if s.transport.is_null() {
        "zenoh".to_owned()
    } else {
        unsafe { CStr::from_ptr(s.transport) }.to_string_lossy().into_owned()
    };

    // We MUST keep the CString alive for the pointer!
    let router_env = std::env::var("VIRTMCU_ZENOH_ROUTER").ok();
    let router_cstring = if !s.router.is_null() {
        None
    } else if let Some(r) = router_env {
        alloc::ffi::CString::new(r).ok()
    } else {
        None
    };

    let router_ptr = if !s.router.is_null() {
        s.router.cast_const()
    } else if let Some(ref c) = router_cstring {
        c.as_ptr()
    } else {
        ptr::null()
    };

    let topic = if s.topic.is_null() {
        None
    } else {
        Some(unsafe { CStr::from_ptr(s.topic) }.to_string_lossy().into_owned())
    };

    s.rust_state =
        ieee802154_init_internal(s, s.irq, s.node_id, &node, transport_name, router_ptr, topic);
    if s.rust_state.is_null() {
        error_setg!(errp, "Failed to initialize Rust Virtmcu 802.15.4");
    }
}

extern "C" fn ieee802154_instance_finalize(obj: *mut Object) {
    let s = unsafe { &mut *(obj as *mut Virtmcu802154QEMU) };
    if !s.rust_state.is_null() {
        ieee802154_cleanup_internal(s.rust_state);
        s.rust_state = ptr::null_mut();
    }
}

extern "C" fn ieee802154_instance_init(obj: *mut Object) {
    let s = unsafe { &mut *(obj as *mut Virtmcu802154QEMU) };

    unsafe {
        memory_region_init_io(
            &raw mut s.iomem,
            obj,
            &raw const VIRTM_802154_OPS,
            obj as *mut c_void,
            c"ieee802154".as_ptr(),
            IEEE_MMIO_SIZE,
        );
    }
    unsafe {
        sysbus_init_mmio(obj as *mut SysBusDevice, &raw mut s.iomem);
    }
    unsafe {
        sysbus_init_irq(obj as *mut SysBusDevice, &raw mut s.irq);
    }
}

define_properties!(
    VIRTM_802154_PROPERTIES,
    [
        define_prop_uint32!(c"node".as_ptr(), Virtmcu802154QEMU, node_id, 0),
        define_prop_string!(c"transport".as_ptr(), Virtmcu802154QEMU, transport),
        define_prop_string!(c"router".as_ptr(), Virtmcu802154QEMU, router),
        define_prop_string!(c"topic".as_ptr(), Virtmcu802154QEMU, topic),
        virtmcu_qom::define_prop_bool!(c"debug".as_ptr(), Virtmcu802154QEMU, debug, false),
    ]
);

extern "C" fn ieee802154_reset(dev: *mut c_void) {
    let s = unsafe { &mut *(dev as *mut Virtmcu802154QEMU) };
    if s.rust_state.is_null() {
        return;
    }
    let state = unsafe { &mut *s.rust_state };
    let mut inner = state.inner.get_mut();

    inner.tx_len = 0;
    inner.rx_len = 0;
    inner.rx_read_pos = 0;
    inner.rx_rssi = 0;
    inner.status = 0;
    inner.state = RadioState::Idle;
    inner.rx_queue.clear();
    inner.nb = 0;
    inner.be = MAC_MIN_BE;
    inner.ack_pending = false;

    if let Some(timer) = &state.rx_timer {
        timer.del();
    }
    if let Some(timer) = &state.backoff_timer {
        timer.del();
    }
    if let Some(timer) = &state.ack_timer {
        timer.del();
    }
    if let Some(timer) = &state.tx_timer {
        timer.del();
    }
}

extern "C" fn ieee802154_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let dc = device_class!(klass);
    unsafe {
        (*dc).realize = Some(ieee802154_realize);
    }
    unsafe {
        (*dc).legacy_reset = Some(ieee802154_reset);
    }
    unsafe {
        (*dc).user_creatable = true;
    }
    virtmcu_qom::device_class_set_props!(dc, VIRTM_802154_PROPERTIES);
}

#[used]
static VIRTM_802154_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"ieee802154".as_ptr(),
    parent: c"sys-bus-device".as_ptr(),
    instance_size: core::mem::size_of::<Virtmcu802154QEMU>(),
    instance_align: 0,
    instance_init: Some(ieee802154_instance_init),
    instance_post_init: None,
    instance_finalize: Some(ieee802154_instance_finalize),
    abstract_: false,
    class_size: core::mem::size_of::<virtmcu_qom::qdev::SysBusDeviceClass>(),
    class_init: Some(ieee802154_class_init),
    class_base_init: None,
    class_data: ptr::null(),
    interfaces: ptr::null(),
};

declare_device_type!(VIRTM_802154_TYPE_INIT, VIRTM_802154_TYPE_INFO);

/* ── Internal Logic ───────────────────────────────────────────────────────── */

fn ieee802154_init_internal(
    parent: *mut Virtmcu802154QEMU,
    irq: QemuIrq,
    node_id: u32,
    node: &str,
    transport_name: String,
    router: *const c_char,
    topic: Option<String>,
) -> *mut Virtmcu802154State {
    let transport: Arc<dyn virtmcu_api::DataTransport> = if transport_name == "unix" {
        let path = if router.is_null() {
            format!("/tmp/virtmcu-coord-{}.sock", { node }) // virtmcu-allow: absolute_path reasoning="Legacy script"
        } else {
            unsafe { core::ffi::CStr::from_ptr(router).to_string_lossy().into_owned() }
        };
        match transport_unix::UnixDataTransport::new(&path) {
            Ok(t) => Arc::new(t),
            Err(e) => {
                virtmcu_qom::sim_err!("FAILED to open unix socket {}: {}", path, e);
                return ptr::null_mut();
            }
        }
    } else {
        match unsafe { transport_zenoh::get_or_init_session(router) } {
            Ok(session) => Arc::new(transport_zenoh::ZenohDataTransport::new(session)),
            Err(e) => {
                virtmcu_qom::sim_err!("FAILED to open Zenoh session: {e}");
                return ptr::null_mut();
            }
        }
    };

    let topic_tx;
    let topic_rx;
    if let Some(t) = topic {
        topic_tx = alloc::format!("{t}/tx");
        topic_rx = alloc::format!("{t}/rx");
    } else {
        topic_tx = alloc::format!("sim/rf/ieee802154/{node_id}/tx");
        topic_rx = alloc::format!("sim/rf/ieee802154/{node_id}/rx");
    }

    let mut state_box = Box::new(Virtmcu802154State {
        _liveliness: None,
        parent_ptr: parent,
        irq,
        transport: Arc::clone(&transport),
        topic_tx: topic_tx.clone(),
        subscription: None,
        rx_timer: None,
        backoff_timer: None,
        ack_timer: None,
        tx_timer: None,
        inner: BqlGuarded::new(Virtmcu802154Inner {
            node_id,
            tx_fifo: [0; IEEE_FIFO_SIZE],
            tx_len: 0,
            rx_fifo: [0; IEEE_FIFO_SIZE],
            rx_len: 0,
            rx_read_pos: 0,
            rx_rssi: 0,
            status: 0,
            state: RadioState::Idle,
            pan_id: IEEE_BROADCAST_PAN,
            short_addr: IEEE_BROADCAST_ADDR,
            ext_addr: 0,
            rx_queue: Vec::with_capacity(IEEE_RX_QUEUE_SIZE),
            nb: 0,
            be: IEEE_DEFAULT_BE,
            ack_pending: false,
            ack_seq: 0,
            tx_sequence: 0,
        }),
    });

    let state_ptr = core::ptr::from_mut(&mut *state_box);
    let state_ptr_usize = state_ptr as usize;

    let sub_callback: virtmcu_api::DataCallback = Box::new(move |_topic: &str, data: &[u8]| {
        let state = unsafe { &mut *(state_ptr_usize as *mut Virtmcu802154State) };
        on_rx_frame(state, data);
    });

    let generation = Arc::new(core::sync::atomic::AtomicU64::new(0));
    state_box.subscription =
        virtmcu_qom::sync::SafeSubscription::new(&*transport, &topic_rx, generation, sub_callback) // virtmcu-allow: bql reasoning="Safe Zenoh integration"
            .ok();

    state_box.rx_timer =
        Some(unsafe { QomTimer::new(QEMU_CLOCK_VIRTUAL, rx_timer_cb, state_ptr as *mut c_void) });

    state_box.backoff_timer = Some(unsafe {
        QomTimer::new(QEMU_CLOCK_VIRTUAL, backoff_timer_cb, state_ptr as *mut c_void)
    });

    state_box.ack_timer =
        Some(unsafe { QomTimer::new(QEMU_CLOCK_VIRTUAL, ack_timer_cb, state_ptr as *mut c_void) });
    state_box.tx_timer =
        Some(unsafe { QomTimer::new(QEMU_CLOCK_VIRTUAL, tx_timer_cb, state_ptr as *mut c_void) });

    state_box._liveliness = if transport_name == "zenoh" {
        match unsafe { transport_zenoh::get_or_init_session(router) } {
            Ok(session) => {
                let hb_topic = format!("sim/ieee802154/liveliness/{node_id}");
                session.liveliness().declare_token(hb_topic).wait().ok()
            }
            Err(_) => None,
        }
    } else {
        None
    };

    Box::into_raw(state_box)
}

fn ieee802154_read_internal(s: &mut Virtmcu802154State, offset: u64) -> u64 {
    let mut inner = s.inner.get_mut();
    match offset {
        REG_TX_LEN => u64::from(inner.tx_len),
        REG_RX_DATA
            if (inner.status & STATUS_RX_PENDING != 0) && (inner.rx_read_pos < inner.rx_len) =>
        {
            let val = u64::from(inner.rx_fifo[inner.rx_read_pos as usize]);
            inner.rx_read_pos += 1;
            val
        }
        REG_RX_LEN => u64::from(inner.rx_len),
        REG_STATUS => u64::from(inner.status | ((inner.state as u32) << STATE_SHIFT)),
        REG_RSSI => u64::from(inner.rx_rssi as u8),
        REG_STATE => inner.state as u64,
        REG_PAN_ID => u64::from(inner.pan_id),
        REG_SHORT_ADDR => u64::from(inner.short_addr),
        REG_EXT_ADDR_LO => inner.ext_addr & ADDR_32_MASK,
        REG_EXT_ADDR_HI => inner.ext_addr >> ADDR_32_SHIFT,
        _ => {
            let parent = unsafe { &*s.parent_ptr };
            if parent.debug {
                virtmcu_qom::sim_warn!("ieee802154_read: unhandled offset 0x{:x}", offset);
            }
            0
        }
    }
}

fn ieee802154_write_internal(s: &mut Virtmcu802154State, offset: u64, value: u64) {
    let mut inner = s.inner.get_mut();
    match offset {
        REG_TX_DATA if inner.tx_len < IEEE_FIFO_SIZE as u32 => {
            let tx_pos = inner.tx_len as usize;
            inner.tx_fifo[tx_pos] = value as u8;
            inner.tx_len += 1;
        }
        REG_TX_LEN => {
            inner.tx_len = (value & TX_LEN_MASK) as u32;
        }
        REG_TX_GO => {
            tx_go(s.irq, s.backoff_timer.as_ref(), &mut inner);
        }
        REG_STATUS => {
            inner.status &= !(value as u32);
            if inner.status & STATUS_RX_PENDING == 0 {
                unsafe { qemu_set_irq(s.irq, 0) };
                check_rx_queue(s.irq, s.rx_timer.as_ref(), &mut inner);
            }
        }
        REG_STATE => {
            let next_state = match value {
                RADIO_STATE_OFF => RadioState::Off,
                RADIO_STATE_IDLE => RadioState::Idle,
                RADIO_STATE_RX => RadioState::Rx,
                RADIO_STATE_TX => RadioState::Tx,
                _ => inner.state,
            };
            if next_state == RadioState::Tx {
                tx_go(s.irq, s.backoff_timer.as_ref(), &mut inner);
            } else {
                inner.state = next_state;
            }
        }
        REG_PAN_ID => {
            inner.pan_id = value as u16;
        }
        REG_SHORT_ADDR => {
            inner.short_addr = value as u16;
        }
        REG_EXT_ADDR_LO => {
            inner.ext_addr =
                (inner.ext_addr & (ADDR_32_MASK << ADDR_32_SHIFT)) | (value & ADDR_32_MASK);
        }
        REG_EXT_ADDR_HI => {
            inner.ext_addr =
                (inner.ext_addr & ADDR_32_MASK) | ((value & ADDR_32_MASK) << ADDR_32_SHIFT);
        }
        _ => {
            let parent = unsafe { &*s.parent_ptr };
            if parent.debug {
                virtmcu_qom::sim_warn!(
                    "ieee802154_write: unhandled offset 0x{:x} val=0x{:x}",
                    offset,
                    value
                );
            }
        }
    }
}

fn ieee802154_cleanup_internal(state: *mut Virtmcu802154State) {
    if state.is_null() {
        return;
    }
    let mut s = unsafe { Box::from_raw(state) };
    s.subscription.take();
    s.rx_timer.take();
    s.backoff_timer.take();
    s.ack_timer.take();
    s.tx_timer.take();
}

const MAC_MAX_BE: u8 = 5;
const MAC_MAX_CSMA_BACKOFFS: u8 = 4;

fn tx_go(_irq: QemuIrq, backoff_timer: Option<&QomTimer>, inner: &mut Virtmcu802154Inner) {
    if inner.state == RadioState::Tx {
        return;
    }
    inner.nb = 0;
    inner.be = MAC_MIN_BE;
    inner.state = RadioState::Tx;
    schedule_backoff(backoff_timer, inner);
}

fn deterministic_random(node_id: u32, vtime_ns: u64, extra: u64) -> u32 {
    let mut hash = FNV_OFFSET_BASIS;
    let mut bytes = [0u8; HASH_BYTES_LEN];
    bytes[NODE_ID_OFFSET..NODE_ID_OFFSET + NODE_ID_SIZE].copy_from_slice(&node_id.to_le_bytes());
    bytes[VTIME_OFFSET..VTIME_OFFSET + VTIME_SIZE].copy_from_slice(&vtime_ns.to_le_bytes());
    bytes[EXTRA_OFFSET..EXTRA_OFFSET + EXTRA_SIZE].copy_from_slice(&extra.to_le_bytes());
    for byte in bytes {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

fn schedule_backoff(backoff_timer: Option<&QomTimer>, inner: &mut Virtmcu802154Inner) {
    let max_backoff = (1u32 << inner.be) - 1;
    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;
    let rand_val = deterministic_random(inner.node_id, now, inner.tx_sequence);
    let backoff_count = rand_val % (max_backoff + 1);
    let delay_ns = u64::from(backoff_count) * UNIT_BACKOFF_PERIOD_NS;

    if let Some(timer) = backoff_timer {
        timer.mod_ns((now + delay_ns) as i64);
    }
}

fn tx_real(
    transport: &dyn virtmcu_api::DataTransport,
    topic: &str,
    tx_timer: Option<&QomTimer>,
    inner: &mut Virtmcu802154Inner,
) {
    let vtime = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;
    let seq = inner.tx_sequence;
    inner.tx_sequence += 1;
    let payload = &inner.tx_fifo[..inner.tx_len as usize];
    let mhr = Rf802154Mhr::parse(payload);
    let msg = virtmcu_api::encode_rf802154_frame(vtime, seq, payload, 0, IEEE_DEFAULT_LQI, mhr);

    let _ = transport.publish(topic, &msg);
    let air_time_ns = (IEEE_OVERHEAD_BYTES + inner.tx_len as u64) * IEEE_NS_PER_BYTE;

    if let Some(timer) = tx_timer {
        timer.mod_ns((vtime + air_time_ns) as i64);
    }
}

extern "C" fn tx_timer_cb(opaque: *mut c_void) {
    let s = unsafe { &mut *(opaque as *mut Virtmcu802154State) };
    let mut inner = s.inner.get_mut();

    inner.tx_len = 0;
    inner.status |= STATUS_TX_DONE;
    inner.state = RadioState::Idle;
    unsafe {
        qemu_set_irq(s.irq, 1);
    }
}

extern "C" fn backoff_timer_cb(opaque: *mut c_void) {
    let s = unsafe { &mut *(opaque as *mut Virtmcu802154State) };
    let mut inner = s.inner.get_mut();
    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;
    let busy = !inner.rx_queue.is_empty() && inner.rx_queue[0].delivery_vtime <= now;

    if busy {
        inner.nb += 1;
        if inner.nb > MAC_MAX_CSMA_BACKOFFS {
            inner.tx_len = 0;
            inner.state = RadioState::Idle;
            inner.status |= STATUS_TX_DONE;
            unsafe {
                qemu_set_irq(s.irq, 1);
            }
        } else {
            inner.be = core::cmp::min(inner.be + 1, MAC_MAX_BE);
            schedule_backoff(s.backoff_timer.as_ref(), &mut inner);
        }
    } else {
        tx_real(&*s.transport, &s.topic_tx, s.tx_timer.as_ref(), &mut inner);
    }
}

extern "C" fn ack_timer_cb(opaque: *mut c_void) {
    let s = unsafe { &mut *(opaque as *mut Virtmcu802154State) };
    let mut inner = s.inner.get_mut();

    if !inner.ack_pending {
        return;
    }

    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;
    let seq = inner.tx_sequence;
    inner.tx_sequence += 1;
    let ack_payload = [IEEE_ACK_FRAME_TYPE, ACK_RESERVED_BYTE, inner.ack_seq];
    let mhr = Rf802154Mhr::parse(&ack_payload);
    let msg = virtmcu_api::encode_rf802154_frame(now, seq, &ack_payload, 0, IEEE_DEFAULT_LQI, mhr);

    let _ = s.transport.publish(&s.topic_tx, &msg);
    inner.ack_pending = false;
}

fn on_rx_frame(state: &mut Virtmcu802154State, data: &[u8]) {
    let mut inner = state.inner.get_mut();
    if inner.state != RadioState::Rx {
        return;
    }

    let frame = match rf802154::size_prefixed_root_as_rf_802154_frame(data) {
        Ok(f) => f,
        Err(_) => return,
    };

    let vtime = frame.delivery_vtime_ns();
    let sequence = frame.sequence_number();
    let rssi = frame.rssi();

    let mhr = Rf802154Mhr {
        fcf: frame.fcf(),
        seq_num: frame.mhr_seq_num(),
        dest_pan: frame.dest_pan(),
        dest_addr: frame.dest_addr(),
        src_pan: frame.src_pan(),
        src_addr: frame.src_addr(),
    };

    let frame_data = match frame.data() {
        Some(d) => d.bytes(),
        None => return,
    };

    let size = frame_data.len();
    if size > IEEE_FIFO_SIZE {
        return;
    }

    if !frame_matches_address(inner.pan_id, inner.short_addr, inner.ext_addr, &mhr) {
        return;
    }

    if (mhr.fcf & IEEE_ACK_REQUEST_BIT) != 0 {
        inner.ack_pending = true;
        inner.ack_seq = mhr.seq_num;
        if let Some(ack_timer) = &state.ack_timer {
            ack_timer.mod_ns((vtime + SIFS_NS) as i64);
        }
    }

    let mut stored_data = [0u8; IEEE_FIFO_SIZE];
    stored_data[..size].copy_from_slice(frame_data);

    if inner.rx_queue.len() < IEEE_RX_QUEUE_SIZE {
        let pos = inner
            .rx_queue
            .binary_search_by(|probe| match probe.delivery_vtime.cmp(&vtime) {
                Ordering::Equal => probe.sequence.cmp(&sequence),
                ord => ord,
            })
            .unwrap_or_else(|e| e);
        inner.rx_queue.insert(
            pos,
            RxFrame { delivery_vtime: vtime, sequence, data: stored_data, size, rssi },
        );

        if let Some(rx_timer) = &state.rx_timer {
            rx_timer.mod_ns(inner.rx_queue[0].delivery_vtime as i64);
        }
    }
}

fn frame_matches_address(pan_id: u16, short_addr: u16, ext_addr: u64, mhr: &Rf802154Mhr) -> bool {
    let dest_addr_mode = (mhr.fcf >> IEEE_ADDR_MODE_SHIFT) & IEEE_ADDR_MODE_MASK;

    match dest_addr_mode {
        IEEE_ADDR_MODE_NONE => true,
        IEEE_ADDR_MODE_SHORT => {
            let pan_matches = mhr.dest_pan == IEEE_BROADCAST_PAN || mhr.dest_pan == pan_id;
            let addr_matches =
                mhr.dest_addr as u16 == IEEE_BROADCAST_ADDR || mhr.dest_addr as u16 == short_addr;
            pan_matches && addr_matches
        }
        IEEE_ADDR_MODE_EXT => {
            let pan_matches = mhr.dest_pan == IEEE_BROADCAST_PAN || mhr.dest_pan == pan_id;
            let addr_matches = mhr.dest_addr == ext_addr;
            pan_matches && addr_matches
        }
        _ => false,
    }
}

fn check_rx_queue(irq: QemuIrq, rx_timer: Option<&QomTimer>, inner: &mut Virtmcu802154Inner) {
    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;
    if !inner.rx_queue.is_empty() {
        if inner.rx_queue[0].delivery_vtime <= now {
            if inner.status & STATUS_RX_PENDING == 0 {
                let frame = inner.rx_queue.remove(0);
                inner.rx_fifo[..frame.size].copy_from_slice(&frame.data[..frame.size]);
                inner.rx_len = frame.size as u32;
                inner.rx_rssi = frame.rssi;
                inner.rx_read_pos = 0;
                inner.status |= STATUS_RX_PENDING;
                unsafe { qemu_set_irq(irq, 1) };

                if !inner.rx_queue.is_empty() {
                    if let Some(timer) = rx_timer {
                        timer.mod_ns(inner.rx_queue[0].delivery_vtime as i64);
                    }
                }
            }
        } else if let Some(timer) = rx_timer {
            timer.mod_ns(inner.rx_queue[0].delivery_vtime as i64);
        }
    }
}

extern "C" fn rx_timer_cb(opaque: *mut c_void) {
    let state = unsafe { &mut *(opaque as *mut Virtmcu802154State) };
    let mut inner = state.inner.get_mut();
    check_rx_queue(state.irq, state.rx_timer.as_ref(), &mut inner);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_802154_qemu_layout() {
        assert_eq!(
            core::mem::offset_of!(Virtmcu802154QEMU, parent_obj),
            0,
            "SysBusDevice must be the first field"
        );
    }
}
