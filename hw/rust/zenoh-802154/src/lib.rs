#![no_std]
//! Zenoh-based 802.15.4 radio for VirtMCU.

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
use virtmcu_qom::sync::BqlGuarded;
use virtmcu_qom::timer::{qemu_clock_get_ns, QomTimer, QEMU_CLOCK_VIRTUAL};
use virtmcu_qom::{
    declare_device_type, define_prop_string, define_prop_uint32, define_properties, device_class,
    error_setg,
};
use virtmcu_zenoh::SafeSubscriber;
use zenoh::pubsub::Publisher;
use zenoh::Session;
use zenoh::Wait;

use core::cmp::Ordering;

#[repr(C)]
pub struct Zenoh802154QEMU {
    pub parent_obj: SysBusDevice,
    pub iomem: MemoryRegion,
    pub irq: QemuIrq,

    /* Properties */
    pub node_id: u32,
    pub router: *mut c_char,
    pub topic: *mut c_char,

    /* Rust state */
    pub rust_state: *mut Zenoh802154State,
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

pub struct Zenoh802154State {
    irq: QemuIrq,
    _session: Arc<Session>,
    // Safety: same as zenoh-chardev — publisher holds Arc back to session; both live in
    // this struct; drop order (top-to-bottom) ensures session outlives publisher.
    publisher: Publisher<'static>,
    subscriber: Option<SafeSubscriber>,

    rx_timer: Option<QomTimer>,
    backoff_timer: Option<QomTimer>,
    ack_timer: Option<QomTimer>,

    // All state accessed exclusively under BQL; see BqlGuarded docs.
    inner: BqlGuarded<Zenoh802154Inner>,
}

struct Zenoh802154Inner {
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

unsafe extern "C" fn zenoh_802154_read(opaque: *mut c_void, offset: u64, _size: c_uint) -> u64 {
    // SAFETY: opaque is a valid pointer provided by QEMU.
    let s = unsafe { &mut *(opaque as *mut Zenoh802154QEMU) };
    if s.rust_state.is_null() {
        return 0;
    }
    // SAFETY: rust_state is non-null.
    unsafe { zenoh_802154_read_internal(&mut *s.rust_state, offset) }
}

unsafe extern "C" fn zenoh_802154_write(
    opaque: *mut c_void,
    offset: u64,
    value: u64,
    _size: c_uint,
) {
    // SAFETY: opaque is a valid pointer provided by QEMU.
    let s = unsafe { &mut *(opaque as *mut Zenoh802154QEMU) };
    if s.rust_state.is_null() {
        return;
    }
    // SAFETY: rust_state is non-null.
    unsafe { zenoh_802154_write_internal(&mut *s.rust_state, offset, value) };
}

static ZENOH_802154_OPS: MemoryRegionOps = MemoryRegionOps {
    read: Some(zenoh_802154_read),
    write: Some(zenoh_802154_write),
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

unsafe extern "C" fn zenoh_802154_realize(dev: *mut c_void, errp: *mut *mut c_void) {
    // SAFETY: dev is a valid pointer.
    let s = unsafe { &mut *(dev as *mut Zenoh802154QEMU) };

    let router_ptr = if s.router.is_null() { ptr::null() } else { s.router.cast_const() };

    let topic = if s.topic.is_null() {
        None
    } else {
        // SAFETY: s.topic is a valid null-terminated C string.
        Some(unsafe { CStr::from_ptr(s.topic) }.to_string_lossy().into_owned())
    };

    s.rust_state = zenoh_802154_init_internal(s.irq, s.node_id, router_ptr, topic);
    if s.rust_state.is_null() {
        error_setg!(errp, "Failed to initialize Rust Zenoh 802.15.4");
    }
}

unsafe extern "C" fn zenoh_802154_instance_finalize(obj: *mut Object) {
    // SAFETY: obj is a valid pointer.
    let s = unsafe { &mut *(obj as *mut Zenoh802154QEMU) };
    if !s.rust_state.is_null() {
        zenoh_802154_cleanup_internal(s.rust_state);
        s.rust_state = ptr::null_mut();
    }
}

unsafe extern "C" fn zenoh_802154_instance_init(obj: *mut Object) {
    // SAFETY: obj is a valid pointer.
    let s = unsafe { &mut *(obj as *mut Zenoh802154QEMU) };

    // SAFETY: s.iomem and obj are valid.
    unsafe {
        memory_region_init_io(
            &raw mut s.iomem,
            obj,
            &raw const ZENOH_802154_OPS,
            obj as *mut c_void,
            c"zenoh-802154".as_ptr(),
            0x100,
        );
        sysbus_init_mmio(obj as *mut SysBusDevice, &raw mut s.iomem);
        sysbus_init_irq(obj as *mut SysBusDevice, &raw mut s.irq);
    }
}

define_properties!(
    ZENOH_802154_PROPERTIES,
    [
        define_prop_uint32!(c"node".as_ptr(), Zenoh802154QEMU, node_id, 0),
        define_prop_string!(c"router".as_ptr(), Zenoh802154QEMU, router),
        define_prop_string!(c"topic".as_ptr(), Zenoh802154QEMU, topic),
    ]
);

unsafe extern "C" fn zenoh_802154_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let dc = device_class!(klass);
    // SAFETY: Setting hooks during class init is safe.
    unsafe {
        (*dc).realize = Some(zenoh_802154_realize);
        (*dc).user_creatable = true;
    }
    virtmcu_qom::device_class_set_props!(dc, ZENOH_802154_PROPERTIES);
}

static ZENOH_802154_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"zenoh-802154".as_ptr(),
    parent: c"sys-bus-device".as_ptr(),
    instance_size: core::mem::size_of::<Zenoh802154QEMU>(),
    instance_align: 0,
    instance_init: Some(zenoh_802154_instance_init),
    instance_post_init: None,
    instance_finalize: Some(zenoh_802154_instance_finalize),
    abstract_: false,
    class_size: 0,
    class_init: Some(zenoh_802154_class_init),
    class_base_init: None,
    class_data: ptr::null(),
    interfaces: ptr::null(),
};

declare_device_type!(ZENOH_802154_TYPE_INIT, ZENOH_802154_TYPE_INFO);

/* ── Internal Logic ───────────────────────────────────────────────────────── */

fn zenoh_802154_init_internal(
    irq: QemuIrq,
    node_id: u32,
    router: *const c_char,
    topic: Option<String>,
) -> *mut Zenoh802154State {
    // SAFETY: get_or_init_session is safe with valid router pointer or null.
    // Safety: router validity is guaranteed by the caller.
    let session = match unsafe { virtmcu_zenoh::get_or_init_session(router) } {
        Ok(s) => s,
        Err(e) => {
            virtmcu_qom::vlog!("[zenoh-802154] node={node_id}: FAILED to open Zenoh session: {e}");
            return ptr::null_mut();
        }
    };

    let topic_tx;
    let topic_rx;
    if let Some(t) = topic {
        topic_tx = alloc::format!("{t}/tx");
        topic_rx = alloc::format!("{t}/rx");
    } else {
        topic_tx = alloc::format!("sim/rf/802154/{node_id}/tx");
        topic_rx = alloc::format!("sim/rf/802154/{node_id}/rx");
    }

    let publisher = match session.declare_publisher(topic_tx).wait() {
        Ok(p) => p,
        Err(e) => {
            virtmcu_qom::vlog!("[zenoh-802154] node={node_id}: FAILED to declare publisher: {e}");
            return ptr::null_mut();
        }
    };

    // Two-phase init: allocate first for a stable address the subscriber captures,
    // then write the constructed state.
    let state_ptr_raw: *mut Zenoh802154State =
        Box::into_raw(Box::<core::mem::MaybeUninit<Zenoh802154State>>::new_uninit()).cast();
    let state_ptr_usize = state_ptr_raw as usize;

    let subscriber = SafeSubscriber::new(&session, &topic_rx, move |sample| {
        // SafeSubscriber holds the BQL and prevents execution if dropped
        // SAFETY: state_ptr_usize is a valid pointer to Zenoh802154State.
        let state = unsafe { &mut *(state_ptr_usize as *mut Zenoh802154State) };
        on_rx_frame(state, sample);
    })
    .ok();

    // SAFETY: creating timers is safe.
    let rx_timer =
        unsafe { QomTimer::new(QEMU_CLOCK_VIRTUAL, rx_timer_cb, state_ptr_raw as *mut c_void) };

    // SAFETY: creating timers is safe.
    let backoff_timer = unsafe {
        QomTimer::new(QEMU_CLOCK_VIRTUAL, backoff_timer_cb, state_ptr_raw as *mut c_void)
    };

    // SAFETY: creating timers is safe.
    let ack_timer =
        unsafe { QomTimer::new(QEMU_CLOCK_VIRTUAL, ack_timer_cb, state_ptr_raw as *mut c_void) };

    let inner = Zenoh802154Inner {
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

    let state = Zenoh802154State {
        irq,
        _session: session,
        publisher,
        subscriber,
        rx_timer: Some(rx_timer),
        backoff_timer: Some(backoff_timer),
        ack_timer: Some(ack_timer),
        inner: BqlGuarded::new(inner),
    };

    // SAFETY: state_ptr_raw is valid.
    unsafe { ptr::write(state_ptr_raw, state) };

    state_ptr_raw
}

fn zenoh_802154_read_internal(s: &mut Zenoh802154State, offset: u64) -> u64 {
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
        _ => 0,
    }
}

fn zenoh_802154_write_internal(s: &mut Zenoh802154State, offset: u64, value: u64) {
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
            // TX GO (legacy)
            tx_go(s.irq, s.backoff_timer.as_ref(), &mut inner);
        }
        0x14 => {
            inner.status &= !(value as u32);
            if inner.status & 0x01 == 0 {
                // SAFETY: s.irq is valid.
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
        _ => {}
    }
}

fn zenoh_802154_cleanup_internal(state: *mut Zenoh802154State) {
    if state.is_null() {
        return;
    }
    // SAFETY: state was allocated via Box::into_raw.
    let mut s = unsafe { Box::from_raw(state) };

    // Explicitly drop the subscriber first to wait for callbacks
    s.subscriber.take();
    s.rx_timer.take();
    s.backoff_timer.take();
    s.ack_timer.take();
}

const UNIT_BACKOFF_PERIOD_NS: u64 = 320_000;
const SIFS_NS: u64 = 192_000;
const MAC_MIN_BE: u8 = 3;
const MAC_MAX_BE: u8 = 5;
const MAC_MAX_CSMA_BACKOFFS: u8 = 4;

fn tx_go(_irq: QemuIrq, backoff_timer: Option<&QomTimer>, inner: &mut Zenoh802154Inner) {
    inner.nb = 0;
    inner.be = MAC_MIN_BE;
    inner.state = RadioState::Tx;
    schedule_backoff(backoff_timer, inner);
}

fn schedule_backoff(backoff_timer: Option<&QomTimer>, inner: &mut Zenoh802154Inner) {
    let max_backoff = (1u32 << inner.be) - 1;
    let backoff_count = rand::random::<u32>() % (max_backoff + 1);
    let delay_ns = u64::from(backoff_count) * UNIT_BACKOFF_PERIOD_NS;
    // SAFETY: Calling qemu_clock_get_ns is safe under BQL.
    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;

    if let Some(timer) = backoff_timer {
        timer.mod_ns((now + delay_ns) as i64);
    }
}

fn tx_real(irq: QemuIrq, publisher: &Publisher<'static>, inner: &mut Zenoh802154Inner) {
    // SAFETY: Calling qemu_clock_get_ns is safe under BQL.
    let vtime = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;
    let seq = inner.tx_sequence;
    inner.tx_sequence += 1;
    let hdr = rf_header::encode(vtime, seq, inner.tx_len, 0, 255);
    let mut msg = Vec::with_capacity(hdr.len() + inner.tx_len as usize);
    msg.extend_from_slice(&hdr);
    msg.extend_from_slice(&inner.tx_fifo[..inner.tx_len as usize]);

    let _ = publisher.put(msg).wait();

    inner.tx_len = 0;
    inner.status |= 0x02; // TX_DONE
    inner.state = RadioState::Idle;
    // SAFETY: irq is valid.
    unsafe {
        qemu_set_irq(irq, 1);
    }
}

extern "C" fn backoff_timer_cb(opaque: *mut c_void) {
    // SAFETY: opaque is a valid pointer to Zenoh802154State.
    let s = unsafe { &mut *(opaque as *mut Zenoh802154State) };
    let mut inner = s.inner.get_mut();

    // SAFETY: Calling qemu_clock_get_ns is safe under BQL.
    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;
    let busy = !inner.rx_queue.is_empty() && inner.rx_queue[0].delivery_vtime <= now;

    if busy {
        inner.nb += 1;
        if inner.nb > MAC_MAX_CSMA_BACKOFFS {
            inner.tx_len = 0;
            inner.state = RadioState::Idle;
            inner.status |= 0x02;
            // SAFETY: s.irq is valid.
            unsafe {
                qemu_set_irq(s.irq, 1);
            }
        } else {
            inner.be = core::cmp::min(inner.be + 1, MAC_MAX_BE);
            schedule_backoff(s.backoff_timer.as_ref(), &mut inner);
        }
    } else {
        tx_real(s.irq, &s.publisher, &mut inner);
    }
}

extern "C" fn ack_timer_cb(opaque: *mut c_void) {
    // SAFETY: opaque is a valid pointer to Zenoh802154State.
    let s = unsafe { &mut *(opaque as *mut Zenoh802154State) };
    let mut inner = s.inner.get_mut();

    if !inner.ack_pending {
        return;
    }

    // SAFETY: Calling qemu_clock_get_ns is safe under BQL.
    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;
    let seq = inner.tx_sequence;
    inner.tx_sequence += 1;
    // ACK frame: FCF(2) + seq(1) = 3 bytes
    let hdr = rf_header::encode(now, seq, 3, 0, 255);
    let mut msg = Vec::with_capacity(hdr.len() + 3);
    msg.extend_from_slice(&hdr);

    msg.push(0x02); // FCF LSB (Type: ACK)
    msg.push(0x00); // FCF MSB
    msg.push(inner.ack_seq);

    let _ = s.publisher.put(msg).wait();
    inner.ack_pending = false;
}

fn on_rx_frame(state: &mut Zenoh802154State, sample: zenoh::sample::Sample) {
    let mut inner = state.inner.get_mut();
    if inner.state != RadioState::Rx {
        return;
    }

    let payload = sample.payload();
    if payload.len() < rf_header::MIN_ENCODED_BYTES {
        return;
    }

    let bytes = payload.to_bytes();

    // Decode FlatBuffer header; skip malformed frames.
    let (vtime, sequence, raw_size, rssi, _lqi) = match rf_header::decode(&bytes) {
        Some(fields) => fields,
        None => return,
    };
    let size = raw_size as usize;

    // The FlatBuffer header is size-prefixed; its length = 4 + le32 value.
    let hdr_len = if bytes.len() >= 4 {
        4 + u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize
    } else {
        return;
    };

    if size > 128 || bytes.len() < hdr_len + size {
        return;
    }

    let frame_data = &bytes[hdr_len..hdr_len + size];

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

fn check_rx_queue(irq: QemuIrq, rx_timer: Option<&QomTimer>, inner: &mut Zenoh802154Inner) {
    // SAFETY: Calling qemu_clock_get_ns is safe under BQL.
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
                // SAFETY: irq is valid.
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
    // SAFETY: opaque is a valid pointer to Zenoh802154State.
    let state = unsafe { &mut *(opaque as *mut Zenoh802154State) };
    let mut inner = state.inner.get_mut();
    check_rx_queue(state.irq, state.rx_timer.as_ref(), &mut inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use byteorder::{ByteOrder, LittleEndian};

    #[test]
    fn test_address_filtering_broadcast() {
        let pan = 0x1234;
        let short = 0x5678;
        let ext = 0x1122334455667788;

        let mut frame = alloc::vec![0x01, 0x08, 0x00, 0xFF, 0xFF, 0xFF, 0xFF];
        assert!(frame_matches_address(pan, short, ext, &frame), "Broadcast should be accepted");

        frame[5] = 0x78;
        frame[6] = 0x56;
        assert!(
            frame_matches_address(pan, short, ext, &frame),
            "Broadcast PAN, matching short addr"
        );

        frame[3] = 0x34;
        frame[4] = 0x12;
        frame[5] = 0xFF;
        frame[6] = 0xFF;
        assert!(
            frame_matches_address(pan, short, ext, &frame),
            "Matching PAN, broadcast short addr"
        );
    }

    #[test]
    fn test_address_filtering_short() {
        let pan = 0xABCD;
        let short = 0x1234;
        let ext = 0x0;

        let frame = alloc::vec![0x01, 0x08, 0x00, 0xCD, 0xAB, 0x34, 0x12];
        assert!(frame_matches_address(pan, short, ext, &frame), "Exact match should be accepted");

        let frame_wrong_pan = alloc::vec![0x01, 0x08, 0x00, 0x00, 0x00, 0x34, 0x12];
        assert!(
            !frame_matches_address(pan, short, ext, &frame_wrong_pan),
            "Wrong PAN should be rejected"
        );

        let frame_wrong_addr = alloc::vec![0x01, 0x08, 0x00, 0xCD, 0xAB, 0x00, 0x00];
        assert!(
            !frame_matches_address(pan, short, ext, &frame_wrong_addr),
            "Wrong address should be rejected"
        );
    }

    #[test]
    fn test_address_filtering_extended() {
        let pan = 0xABCD;
        let short = 0x1234;
        let ext = 0x1122334455667788;

        let frame = alloc::vec![
            0x01, 0x0C, 0x00, 0xCD, 0xAB, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11
        ];
        assert!(
            frame_matches_address(pan, short, ext, &frame),
            "Exact extended match should be accepted"
        );

        let frame_wrong_pan = alloc::vec![
            0x01, 0x0C, 0x00, 0x00, 0x00, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11
        ];
        assert!(
            !frame_matches_address(pan, short, ext, &frame_wrong_pan),
            "Wrong PAN should be rejected"
        );

        let frame_wrong_addr = alloc::vec![
            0x01, 0x0C, 0x00, 0xCD, 0xAB, 0x00, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11
        ];
        assert!(
            !frame_matches_address(pan, short, ext, &frame_wrong_addr),
            "Wrong extended address should be rejected"
        );
    }

    #[test]
    fn rf_header_encode_decode() {
        let vtime: u64 = 9_876_543_210_000;
        let sequence: u64 = 42;
        let size: u32 = 20;
        let rssi: i8 = -70;
        let buf = rf_header::encode(vtime, sequence, size, rssi, 255);
        let (v2, sq2, s2, r2, _l2) = rf_header::decode(&buf).unwrap();
        assert_eq!(v2, vtime);
        assert_eq!(sq2, sequence);
        assert_eq!(s2, size);
        assert_eq!(r2, rssi);
    }

    #[test]
    fn rx_queue_priority_order() {
        let mut queue: alloc::vec::Vec<RxFrame> = alloc::vec::Vec::new();
        let frames = [(300u64, 0u64), (100u64, 0u64), (200u64, 0u64), (200u64, 1u64)];
        for (vt, sq) in frames {
            let pos = queue
                .binary_search_by(|p| match p.delivery_vtime.cmp(&vt) {
                    Ordering::Equal => p.sequence.cmp(&sq),
                    ord => ord,
                })
                .unwrap_or_else(|e| e);
            queue.insert(
                pos,
                RxFrame { delivery_vtime: vt, sequence: sq, data: [0; 128], size: 0, rssi: 0 },
            );
        }
        assert_eq!(queue[0].delivery_vtime, 100);
        assert_eq!(queue[1].delivery_vtime, 200);
        assert_eq!(queue[1].sequence, 0);
        assert_eq!(queue[2].delivery_vtime, 200);
        assert_eq!(queue[2].sequence, 1);
        assert_eq!(queue[3].delivery_vtime, 300);
    }

    #[test]
    fn test_zenoh_802154_qemu_layout() {
        // QOM layout validation
        assert_eq!(
            core::mem::offset_of!(Zenoh802154QEMU, parent_obj),
            0,
            "SysBusDevice must be the first field"
        );
    }
}
