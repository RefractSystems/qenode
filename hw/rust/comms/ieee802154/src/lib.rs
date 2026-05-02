// std is required: zenoh/tokio bring std
//! Virtmcu 802.15.4 radio with pluggable transport.
use zenoh::Wait;

extern crate alloc;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use byteorder::{ByteOrder, LittleEndian};
use core::ffi::{c_char, c_uint, c_void, CStr};
use core::ptr;
use virtmcu_api::rf_generated::rf_header;
use virtmcu_qom::irq::{qemu_set_irq, QemuIrq};
use virtmcu_qom::memory::{
    memory_region_init_io, MemoryRegion, MemoryRegionOps, DEVICE_LITTLE_ENDIAN,
};
use virtmcu_qom::qdev::{sysbus_init_irq, sysbus_init_mmio, SysBusDevice};
use virtmcu_qom::qom::{Object, ObjectClass, TypeInfo};
use virtmcu_qom::sync::{BqlGuarded, SafeSubscription}; // BQL_EXCEPTION: Safe Zenoh integration
use virtmcu_qom::timer::{qemu_clock_get_ns, QomTimer, QEMU_CLOCK_VIRTUAL};
use virtmcu_qom::{
    declare_device_type, define_prop_string, define_prop_uint32, define_properties, device_class,
    error_setg,
};

use core::cmp::Ordering;

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
    subscription: Option<SafeSubscription>, // BQL_EXCEPTION: SafeSubscription ensures thread safety for Zenoh callbacks

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
        max_access_size: 8,
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
    let router_ptr = if s.router.is_null() { ptr::null() } else { s.router.cast_const() };

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
            0x100,
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
            format!("/tmp/virtmcu-coord-{}.sock", { node })
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

    let state_ptr_raw: *mut Virtmcu802154State =
        Box::into_raw(Box::<core::mem::MaybeUninit<Virtmcu802154State>>::new_uninit()).cast();
    let state_ptr_usize = state_ptr_raw as usize;

    let sub_callback: virtmcu_api::DataCallback = Box::new(move |data| {
        let state = unsafe { &mut *(state_ptr_usize as *mut Virtmcu802154State) };
        on_rx_frame(state, data);
    });

    let generation = Arc::new(core::sync::atomic::AtomicU64::new(0));
    let subscription =
        virtmcu_qom::sync::SafeSubscription::new(&*transport, &topic_rx, generation, sub_callback) // BQL_EXCEPTION: Safe Zenoh integration
            .ok();

    let rx_timer =
        unsafe { QomTimer::new(QEMU_CLOCK_VIRTUAL, rx_timer_cb, state_ptr_raw as *mut c_void) };

    let backoff_timer = unsafe {
        QomTimer::new(QEMU_CLOCK_VIRTUAL, backoff_timer_cb, state_ptr_raw as *mut c_void)
    };

    let ack_timer =
        unsafe { QomTimer::new(QEMU_CLOCK_VIRTUAL, ack_timer_cb, state_ptr_raw as *mut c_void) };
    let tx_timer =
        unsafe { QomTimer::new(QEMU_CLOCK_VIRTUAL, tx_timer_cb, state_ptr_raw as *mut c_void) };

    let inner = Virtmcu802154Inner {
        node_id,
        tx_fifo: [0; 128],
        tx_len: 0,
        rx_fifo: [0; 128],
        rx_len: 0,
        rx_read_pos: 0,
        rx_rssi: 0,
        status: 0,
        state: RadioState::Idle,
        pan_id: 0xFFFF,
        short_addr: 0xFFFF,
        ext_addr: 0,
        rx_queue: Vec::with_capacity(16),
        nb: 0,
        be: 3,
        ack_pending: false,
        ack_seq: 0,
        tx_sequence: 0,
    };

    let liveliness = if transport_name == "zenoh" {
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
    let state = Virtmcu802154State {
        _liveliness: liveliness,
        parent_ptr: parent,
        irq,
        transport,
        topic_tx,
        subscription,
        rx_timer: Some(rx_timer),
        backoff_timer: Some(backoff_timer),
        ack_timer: Some(ack_timer),
        tx_timer: Some(tx_timer),
        inner: BqlGuarded::new(inner),
    };

    unsafe { ptr::write(state_ptr_raw, state) };

    state_ptr_raw
}

fn ieee802154_read_internal(s: &mut Virtmcu802154State, offset: u64) -> u64 {
    let mut inner = s.inner.get_mut();
    match offset {
        0x04 => u64::from(inner.tx_len),
        0x0C if (inner.status & 0x01 != 0) && (inner.rx_read_pos < inner.rx_len) => {
            let val = u64::from(inner.rx_fifo[inner.rx_read_pos as usize]);
            inner.rx_read_pos += 1;
            val
        }
        0x10 => u64::from(inner.rx_len),
        0x14 => u64::from(inner.status | ((inner.state as u32) << 8)),
        0x18 => u64::from(inner.rx_rssi as u8),
        0x1C => inner.state as u64,
        0x20 => u64::from(inner.pan_id),
        0x24 => u64::from(inner.short_addr),
        0x28 => inner.ext_addr & 0xFFFFFFFF,
        0x2C => inner.ext_addr >> 32,
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
        0x00 if inner.tx_len < 128 => {
            let tx_pos = inner.tx_len as usize;
            inner.tx_fifo[tx_pos] = value as u8;
            inner.tx_len += 1;
        }
        0x04 => {
            inner.tx_len = (value & 0x7F) as u32;
        }
        0x08 => {
            tx_go(s.irq, s.backoff_timer.as_ref(), &mut inner);
        }
        0x14 => {
            inner.status &= !(value as u32);
            if inner.status & 0x01 == 0 {
                unsafe { qemu_set_irq(s.irq, 0) };
                check_rx_queue(s.irq, s.rx_timer.as_ref(), &mut inner);
            }
        }
        0x1C => {
            let next_state = match value {
                0 => RadioState::Off,
                1 => RadioState::Idle,
                2 => RadioState::Rx,
                3 => RadioState::Tx,
                _ => inner.state,
            };
            if next_state == RadioState::Tx {
                tx_go(s.irq, s.backoff_timer.as_ref(), &mut inner);
            } else {
                inner.state = next_state;
            }
        }
        0x20 => {
            inner.pan_id = value as u16;
        }
        0x24 => {
            inner.short_addr = value as u16;
        }
        0x28 => {
            inner.ext_addr = (inner.ext_addr & 0xFFFFFFFF00000000) | (value & 0xFFFFFFFF);
        }
        0x2C => {
            inner.ext_addr = (inner.ext_addr & 0x00000000FFFFFFFF) | ((value & 0xFFFFFFFF) << 32);
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

const UNIT_BACKOFF_PERIOD_NS: u64 = 320_000;
const SIFS_NS: u64 = 192_000;
const MAC_MIN_BE: u8 = 3;
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
    let mut hash = 0x811c9dc5u32;
    let mut bytes = [0u8; 20];
    bytes[0..4].copy_from_slice(&node_id.to_le_bytes());
    bytes[4..12].copy_from_slice(&vtime_ns.to_le_bytes());
    bytes[12..20].copy_from_slice(&extra.to_le_bytes());
    for byte in bytes {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(0x01000193);
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
    let hdr = rf_header::encode(vtime, seq, inner.tx_len, 0, 255);
    let mut msg = Vec::with_capacity(hdr.len() + inner.tx_len as usize);
    msg.extend_from_slice(&hdr);
    msg.extend_from_slice(&inner.tx_fifo[..inner.tx_len as usize]);

    let _ = transport.publish(topic, &msg);
    let air_time_ns = (6 + inner.tx_len as u64) * 32_000;

    if let Some(timer) = tx_timer {
        timer.mod_ns((vtime + air_time_ns) as i64);
    }
}

extern "C" fn tx_timer_cb(opaque: *mut c_void) {
    let s = unsafe { &mut *(opaque as *mut Virtmcu802154State) };
    let mut inner = s.inner.get_mut();

    inner.tx_len = 0;
    inner.status |= 0x02;
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
            inner.status |= 0x02;
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
    let hdr = rf_header::encode(now, seq, 3, 0, 255);
    let mut msg = Vec::with_capacity(hdr.len() + 3);
    msg.extend_from_slice(&hdr);

    msg.push(0x02);
    msg.push(0x00);
    msg.push(inner.ack_seq);

    let _ = s.transport.publish(&s.topic_tx, &msg);
    inner.ack_pending = false;
}

fn on_rx_frame(state: &mut Virtmcu802154State, data: &[u8]) {
    let mut inner = state.inner.get_mut();
    if inner.state != RadioState::Rx {
        return;
    }

    if data.len() < rf_header::MIN_ENCODED_BYTES {
        return;
    }

    let (vtime, sequence, raw_size, rssi, _lqi) = match rf_header::decode(data) {
        Some(fields) => fields,
        None => return,
    };
    let size = raw_size as usize;

    let hdr_len = if data.len() >= 4 {
        4 + u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize
    } else {
        return;
    };

    if size > 128 || data.len() < hdr_len + size {
        return;
    }

    let frame_data = &data[hdr_len..hdr_len + size];

    if !frame_matches_address(inner.pan_id, inner.short_addr, inner.ext_addr, frame_data) {
        return;
    }

    if frame_data.len() >= 3 {
        let fcf = LittleEndian::read_u16(&frame_data[0..2]);
        if (fcf & (1 << 5)) != 0 {
            inner.ack_pending = true;
            inner.ack_seq = frame_data[2];
            if let Some(ack_timer) = &state.ack_timer {
                ack_timer.mod_ns((vtime + SIFS_NS) as i64);
            }
        }
    }

    let mut stored_data = [0u8; 128];
    stored_data[..size].copy_from_slice(frame_data);

    if inner.rx_queue.len() < 16 {
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

fn frame_matches_address(pan_id: u16, short_addr: u16, ext_addr: u64, frame: &[u8]) -> bool {
    if frame.len() < 3 {
        return false;
    }

    let fcf = LittleEndian::read_u16(&frame[0..2]);
    let dest_addr_mode = (fcf >> 10) & 0x03;

    match dest_addr_mode {
        0x00 => true,
        0x02 => {
            if frame.len() < 7 {
                return false;
            }
            let dest_pan = LittleEndian::read_u16(&frame[3..5]);
            let dest_addr = LittleEndian::read_u16(&frame[5..7]);
            let pan_matches = dest_pan == 0xFFFF || dest_pan == pan_id;
            let addr_matches = dest_addr == 0xFFFF || dest_addr == short_addr;
            pan_matches && addr_matches
        }
        0x03 => {
            if frame.len() < 13 {
                return false;
            }
            let dest_pan = LittleEndian::read_u16(&frame[3..5]);
            let dest_addr = LittleEndian::read_u64(&frame[5..13]);
            let pan_matches = dest_pan == 0xFFFF || dest_pan == pan_id;
            let addr_matches = dest_addr == ext_addr;
            pan_matches && addr_matches
        }
        _ => false,
    }
}

fn check_rx_queue(irq: QemuIrq, rx_timer: Option<&QomTimer>, inner: &mut Virtmcu802154Inner) {
    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;
    if !inner.rx_queue.is_empty() {
        if inner.rx_queue[0].delivery_vtime <= now {
            if inner.status & 0x01 == 0 {
                let frame = inner.rx_queue.remove(0);
                inner.rx_fifo[..frame.size].copy_from_slice(&frame.data[..frame.size]);
                inner.rx_len = frame.size as u32;
                inner.rx_rssi = frame.rssi;
                inner.rx_read_pos = 0;
                inner.status |= 0x01;
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
