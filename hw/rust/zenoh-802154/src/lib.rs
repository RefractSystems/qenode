#![allow(unused_variables)]
#![allow(clippy::all)]
#![allow(
    clippy::missing_safety_doc,
    clippy::collapsible_match,
    dead_code,
    unused_imports,
    clippy::needless_return,
    clippy::manual_range_contains,
    clippy::single_component_path_imports,
    clippy::len_zero,
    clippy::while_immutable_condition
)]

use byteorder::{ByteOrder, LittleEndian};
use core::ffi::{c_char, c_uint, c_void};
use std::ffi::{CStr, CString};
use std::ptr;
use virtmcu_qom::error::Error;
use virtmcu_qom::irq::{qemu_irq, qemu_set_irq};
use virtmcu_qom::memory::{
    memory_region_init_io, MemoryRegion, MemoryRegionOps, DEVICE_LITTLE_ENDIAN,
};
use virtmcu_qom::qdev::{sysbus_init_irq, sysbus_init_mmio, DeviceClass, SysBusDevice};
use virtmcu_qom::qom::{Object, ObjectClass, TypeInfo};
use virtmcu_qom::sync::{virtmcu_mutex_free, virtmcu_mutex_new, Bql, QemuMutex};
use virtmcu_qom::timer::{
    qemu_clock_get_ns, virtmcu_timer_free, virtmcu_timer_mod, virtmcu_timer_new_ns, QemuTimer,
    QEMU_CLOCK_VIRTUAL,
};
use virtmcu_qom::{
    declare_device_type, define_prop_string, define_prop_uint32, define_properties, device_class,
    device_class_set_props, error_setg,
};
use zenoh::pubsub::{Publisher, Subscriber};
use zenoh::Session;
use zenoh::Wait;

#[repr(C)]
pub struct Zenoh802154QEMU {
    pub parent_obj: SysBusDevice,
    pub iomem: MemoryRegion,
    pub irq: qemu_irq,

    /* Properties */
    pub node_id: u32,
    pub router: *mut c_char,
    pub topic: *mut c_char,

    /* Rust state */
    pub rust_state: *mut Zenoh802154State,
}

#[repr(C, packed)]
struct ZenohRfHeader {
    delivery_vtime_ns: u64,
    size: u32,
    rssi: i8,
    lqi: u8,
}

struct RxFrame {
    delivery_vtime: u64,
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
    irq: qemu_irq,
    #[allow(dead_code)]
    session: Session,
    // Safety: same as zenoh-chardev — publisher holds Arc back to session; both live in
    // this struct; drop order (top-to-bottom) ensures session outlives publisher.
    publisher: Publisher<'static>,
    #[allow(dead_code)]
    subscriber: Subscriber<()>,

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

    rx_timer: *mut QemuTimer,
    backoff_timer: *mut QemuTimer,
    ack_timer: *mut QemuTimer,
    rx_queue: Vec<RxFrame>,
    mutex: *mut QemuMutex,

    // CSMA/CA state
    nb: u8,
    be: u8,

    // Auto-ACK state
    ack_pending: bool,
    ack_seq: u8,
}

unsafe extern "C" fn zenoh_802154_read(opaque: *mut c_void, offset: u64, _size: c_uint) -> u64 {
    let s = &mut *(opaque as *mut Zenoh802154QEMU);
    if s.rust_state.is_null() {
        return 0;
    }
    zenoh_802154_read_internal(&mut *s.rust_state, offset)
}

unsafe extern "C" fn zenoh_802154_write(
    opaque: *mut c_void,
    offset: u64,
    value: u64,
    _size: c_uint,
) {
    let s = &mut *(opaque as *mut Zenoh802154QEMU);
    if s.rust_state.is_null() {
        return;
    }
    zenoh_802154_write_internal(&mut *s.rust_state, offset, value);
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
    let s = &mut *(dev as *mut Zenoh802154QEMU);

    let router_ptr = if s.router.is_null() {
        ptr::null()
    } else {
        s.router as *const c_char
    };

    let topic = if s.topic.is_null() {
        None
    } else {
        Some(unsafe { CStr::from_ptr(s.topic).to_string_lossy().into_owned() })
    };

    s.rust_state = zenoh_802154_init_internal(s.irq, s.node_id, router_ptr, topic);
    if s.rust_state.is_null() {
        error_setg!(
            errp as *mut *mut Error,
            c"Failed to initialize Rust Zenoh 802.15.4".as_ptr()
        );
        return;
    }
}

unsafe extern "C" fn zenoh_802154_instance_finalize(obj: *mut Object) {
    let s = &mut *(obj as *mut Zenoh802154QEMU);
    if !s.rust_state.is_null() {
        zenoh_802154_cleanup_internal(s.rust_state);
        s.rust_state = ptr::null_mut();
    }
}

unsafe extern "C" fn zenoh_802154_instance_init(obj: *mut Object) {
    let s = &mut *(obj as *mut Zenoh802154QEMU);

    memory_region_init_io(
        &mut s.iomem,
        obj,
        &ZENOH_802154_OPS,
        obj as *mut c_void,
        c"zenoh-802154".as_ptr(),
        0x100,
    );
    sysbus_init_mmio(obj as *mut SysBusDevice, &mut s.iomem);
    sysbus_init_irq(obj as *mut SysBusDevice, &mut s.irq);
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
    unsafe {
        (*dc).realize = Some(zenoh_802154_realize);
        (*dc).user_creatable = true;
    }
    virtmcu_qom::device_class_set_props!(dc, ZENOH_802154_PROPERTIES);
}

static ZENOH_802154_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"zenoh-802154".as_ptr(),
    parent: c"sys-bus-device".as_ptr(),
    instance_size: std::mem::size_of::<Zenoh802154QEMU>(),
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

declare_device_type!(zenoh_802154_type_init, ZENOH_802154_TYPE_INFO);

/* ── Internal Logic ───────────────────────────────────────────────────────── */

fn zenoh_802154_init_internal(
    irq: qemu_irq,
    node_id: u32,
    router: *const c_char,
    topic: Option<String>,
) -> *mut Zenoh802154State {
    let session = match unsafe { virtmcu_zenoh::open_session(router) } {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "[zenoh-802154] node={}: FAILED to open Zenoh session: {}",
                node_id, e
            );
            return ptr::null_mut();
        }
    };

    let topic_tx;
    let topic_rx;
    if let Some(t) = topic {
        topic_tx = format!("{}/tx", t);
        topic_rx = format!("{}/rx", t);
    } else {
        topic_tx = format!("sim/rf/802154/{}/tx", node_id);
        topic_rx = format!("sim/rf/802154/{}/rx", node_id);
    }

    let publisher = session.declare_publisher(topic_tx).wait().unwrap();

    // Two-phase init: allocate first for a stable address the subscriber captures,
    // then write the constructed state.
    let state_ptr_raw: *mut Zenoh802154State =
        Box::into_raw(Box::<std::mem::MaybeUninit<Zenoh802154State>>::new_uninit()).cast();
    let state_ptr_usize = state_ptr_raw as usize;

    let subscriber = session
        .declare_subscriber(topic_rx)
        .callback(move |sample| {
            let state = unsafe { &mut *(state_ptr_usize as *mut Zenoh802154State) };
            on_rx_frame(state, sample);
        })
        .wait()
        .unwrap();

    let rx_timer = unsafe {
        virtmcu_timer_new_ns(
            QEMU_CLOCK_VIRTUAL,
            rx_timer_cb,
            state_ptr_raw as *mut c_void,
        )
    };

    let backoff_timer = unsafe {
        virtmcu_timer_new_ns(
            QEMU_CLOCK_VIRTUAL,
            backoff_timer_cb,
            state_ptr_raw as *mut c_void,
        )
    };

    let ack_timer = unsafe {
        virtmcu_timer_new_ns(
            QEMU_CLOCK_VIRTUAL,
            ack_timer_cb,
            state_ptr_raw as *mut c_void,
        )
    };

    let mutex = unsafe { virtmcu_mutex_new() };

    let state = Zenoh802154State {
        irq,
        session,
        publisher,
        subscriber,
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
        rx_timer,
        backoff_timer,
        ack_timer,
        rx_queue: Vec::with_capacity(16),
        mutex,
        nb: 0,
        be: 3,
        ack_pending: false,
        ack_seq: 0,
    };

    unsafe { ptr::write(state_ptr_raw, state) };

    state_ptr_raw
}

fn zenoh_802154_read_internal(s: &mut Zenoh802154State, offset: u64) -> u64 {
    match offset {
        0x04 => s.tx_len as u64,
        0x0C => {
            if (s.status & 0x01 != 0) && (s.rx_read_pos < s.rx_len) {
                let val = s.rx_fifo[s.rx_read_pos as usize] as u64;
                s.rx_read_pos += 1;
                val
            } else {
                0
            }
        }
        0x10 => s.rx_len as u64,
        0x14 => (s.status | ((s.state as u32) << 8)) as u64,
        0x18 => (s.rx_rssi as u8) as u64,
        0x1C => s.state as u64,
        0x20 => s.pan_id as u64,
        0x24 => s.short_addr as u64,
        0x28 => (s.ext_addr & 0xFFFFFFFF) as u64,
        0x2C => (s.ext_addr >> 32) as u64,
        _ => 0,
    }
}

fn zenoh_802154_write_internal(s: &mut Zenoh802154State, offset: u64, value: u64) {
    match offset {
        0x00 => {
            if s.tx_len < 128 {
                s.tx_fifo[s.tx_len as usize] = value as u8;
                s.tx_len += 1;
            }
        }
        0x04 => {
            s.tx_len = (value & 0x7F) as u32;
        }
        0x08 => {
            // TX GO (legacy)
            tx_go(s);
        }
        0x14 => {
            s.status &= !(value as u32);
            if s.status & 0x01 == 0 {
                unsafe { qemu_set_irq(s.irq, 0) };
                let _guard = unsafe { (*s.mutex).lock() };
                unsafe { check_rx_queue(s) };
            }
        }
        0x1C => {
            let next_state = match value {
                0 => RadioState::Off,
                1 => RadioState::Idle,
                2 => RadioState::Rx,
                3 => RadioState::Tx,
                _ => s.state,
            };
            if next_state == RadioState::Tx {
                tx_go(s);
            } else {
                s.state = next_state;
            }
        }
        0x20 => {
            s.pan_id = value as u16;
        }
        0x24 => {
            s.short_addr = value as u16;
        }
        0x28 => {
            s.ext_addr = (s.ext_addr & 0xFFFFFFFF00000000) | (value & 0xFFFFFFFF);
        }
        0x2C => {
            s.ext_addr = (s.ext_addr & 0x00000000FFFFFFFF) | ((value & 0xFFFFFFFF) << 32);
        }
        _ => {}
    }
}

fn zenoh_802154_cleanup_internal(state: *mut Zenoh802154State) {
    if state.is_null() {
        return;
    }
    let s = unsafe { Box::from_raw(state) };
    unsafe {
        if !s.rx_timer.is_null() {
            virtmcu_timer_free(s.rx_timer);
        }
        if !s.backoff_timer.is_null() {
            virtmcu_timer_free(s.backoff_timer);
        }
        if !s.ack_timer.is_null() {
            virtmcu_timer_free(s.ack_timer);
        }
        virtmcu_mutex_free(s.mutex);
    }
}

const UNIT_BACKOFF_PERIOD_NS: u64 = 320_000;
const SIFS_NS: u64 = 192_000;
const MAC_MIN_BE: u8 = 3;
const MAC_MAX_BE: u8 = 5;
const MAC_MAX_CSMA_BACKOFFS: u8 = 4;

fn tx_go(s: &mut Zenoh802154State) {
    s.nb = 0;
    s.be = MAC_MIN_BE;
    s.state = RadioState::Tx;
    schedule_backoff(s);
}

fn schedule_backoff(s: &mut Zenoh802154State) {
    let max_backoff = (1u32 << s.be) - 1;
    let backoff_count = rand::random::<u32>() % (max_backoff + 1);
    let delay_ns = backoff_count as u64 * UNIT_BACKOFF_PERIOD_NS;
    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;

    unsafe {
        virtmcu_timer_mod(s.backoff_timer, (now + delay_ns) as i64);
    }
}

fn tx_real(s: &mut Zenoh802154State) {
    let vtime = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;
    let mut msg = Vec::with_capacity(14 + s.tx_len as usize);

    let mut hdr_bytes = [0u8; 14];
    LittleEndian::write_u64(&mut hdr_bytes[0..8], vtime);
    LittleEndian::write_u32(&mut hdr_bytes[8..12], s.tx_len);
    hdr_bytes[12] = 0; // RSSI
    hdr_bytes[13] = 255; // LQI

    msg.extend_from_slice(&hdr_bytes);
    msg.extend_from_slice(&s.tx_fifo[..s.tx_len as usize]);

    let _ = s.publisher.put(msg).wait();

    s.tx_len = 0;
    s.status |= 0x02; // TX_DONE
    s.state = RadioState::Idle;
    unsafe {
        qemu_set_irq(s.irq, 1);
    }
}

extern "C" fn backoff_timer_cb(opaque: *mut c_void) {
    let s = unsafe { &mut *(opaque as *mut Zenoh802154State) };
    let _guard = unsafe { (*s.mutex).lock() };

    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;
    let busy = !s.rx_queue.is_empty() && s.rx_queue[0].delivery_vtime <= now;

    if !busy {
        tx_real(s);
    } else {
        s.nb += 1;
        if s.nb > MAC_MAX_CSMA_BACKOFFS {
            s.tx_len = 0;
            s.state = RadioState::Idle;
            s.status |= 0x02;
            unsafe {
                qemu_set_irq(s.irq, 1);
            }
        } else {
            s.be = std::cmp::min(s.be + 1, MAC_MAX_BE);
            schedule_backoff(s);
        }
    }
}

extern "C" fn ack_timer_cb(opaque: *mut c_void) {
    let s = unsafe { &mut *(opaque as *mut Zenoh802154State) };
    let _guard = unsafe { (*s.mutex).lock() };

    if !s.ack_pending {
        return;
    }

    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;
    let mut msg = Vec::with_capacity(14 + 3);

    let mut hdr_bytes = [0u8; 14];
    LittleEndian::write_u64(&mut hdr_bytes[0..8], now);
    LittleEndian::write_u32(&mut hdr_bytes[8..12], 3);
    hdr_bytes[12] = 0; // RSSI
    hdr_bytes[13] = 255; // LQI

    msg.extend_from_slice(&hdr_bytes);

    msg.push(0x02); // FCF LSB (Type: ACK)
    msg.push(0x00); // FCF MSB
    msg.push(s.ack_seq);

    let _ = s.publisher.put(msg).wait();
    s.ack_pending = false;
}

fn on_rx_frame(state: &mut Zenoh802154State, sample: zenoh::sample::Sample) {
    if state.state != RadioState::Rx {
        return;
    }

    let payload = sample.payload();
    if payload.len() < 14 {
        return;
    }

    let bytes = payload.to_bytes();
    let vtime = LittleEndian::read_u64(&bytes[0..8]);
    let size = LittleEndian::read_u32(&bytes[8..12]) as usize;
    let rssi = bytes[12] as i8;

    if size > 128 || bytes.len() < 14 + size {
        return;
    }

    let frame_data = &bytes[14..14 + size];

    if !frame_matches_address(state.pan_id, state.short_addr, state.ext_addr, frame_data) {
        return;
    }

    if frame_data.len() >= 3 {
        let fcf = LittleEndian::read_u16(&frame_data[0..2]);
        if (fcf & (1 << 5)) != 0 {
            state.ack_pending = true;
            state.ack_seq = frame_data[2];
            unsafe {
                virtmcu_timer_mod(state.ack_timer, (vtime + SIFS_NS) as i64);
            }
        }
    }

    let mut stored_data = [0u8; 128];
    stored_data[..size].copy_from_slice(frame_data);

    let _bql_guard = Bql::lock();
    let _mutex_guard = unsafe { (*state.mutex).lock() };

    if state.rx_queue.len() < 16 {
        let pos = state
            .rx_queue
            .binary_search_by(|probe| probe.delivery_vtime.cmp(&vtime))
            .unwrap_or_else(|e| e);
        state.rx_queue.insert(
            pos,
            RxFrame {
                delivery_vtime: vtime,
                data: stored_data,
                size,
                rssi,
            },
        );

        unsafe {
            virtmcu_timer_mod(state.rx_timer, state.rx_queue[0].delivery_vtime as i64);
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

unsafe fn check_rx_queue(s: &mut Zenoh802154State) {
    let now = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) as u64;
    if !s.rx_queue.is_empty() {
        if s.rx_queue[0].delivery_vtime <= now {
            if s.status & 0x01 == 0 {
                let frame = s.rx_queue.remove(0);
                s.rx_fifo[..frame.size].copy_from_slice(&frame.data[..frame.size]);
                s.rx_len = frame.size as u32;
                s.rx_rssi = frame.rssi;
                s.rx_read_pos = 0;

                s.status |= 0x01;
                qemu_set_irq(s.irq, 1);

                if !s.rx_queue.is_empty() {
                    virtmcu_timer_mod(s.rx_timer, s.rx_queue[0].delivery_vtime as i64);
                }
            }
        } else {
            virtmcu_timer_mod(s.rx_timer, s.rx_queue[0].delivery_vtime as i64);
        }
    }
}

extern "C" fn rx_timer_cb(opaque: *mut c_void) {
    let state = unsafe { &mut *(opaque as *mut Zenoh802154State) };
    let _guard = unsafe { (*state.mutex).lock() };
    unsafe { check_rx_queue(state) };
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

        let mut frame = vec![0x01, 0x08, 0x00, 0xFF, 0xFF, 0xFF, 0xFF];
        assert!(
            frame_matches_address(pan, short, ext, &frame),
            "Broadcast should be accepted"
        );

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

        let frame = vec![0x01, 0x08, 0x00, 0xCD, 0xAB, 0x34, 0x12];
        assert!(
            frame_matches_address(pan, short, ext, &frame),
            "Exact match should be accepted"
        );

        let frame_wrong_pan = vec![0x01, 0x08, 0x00, 0x00, 0x00, 0x34, 0x12];
        assert!(
            !frame_matches_address(pan, short, ext, &frame_wrong_pan),
            "Wrong PAN should be rejected"
        );

        let frame_wrong_addr = vec![0x01, 0x08, 0x00, 0xCD, 0xAB, 0x00, 0x00];
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

        let frame = vec![
            0x01, 0x0C, 0x00, 0xCD, 0xAB, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11,
        ];
        assert!(
            frame_matches_address(pan, short, ext, &frame),
            "Exact extended match should be accepted"
        );

        let frame_wrong_pan = vec![
            0x01, 0x0C, 0x00, 0x00, 0x00, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11,
        ];
        assert!(
            !frame_matches_address(pan, short, ext, &frame_wrong_pan),
            "Wrong PAN should be rejected"
        );

        let frame_wrong_addr = vec![
            0x01, 0x0C, 0x00, 0xCD, 0xAB, 0x00, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11,
        ];
        assert!(
            !frame_matches_address(pan, short, ext, &frame_wrong_addr),
            "Wrong extended address should be rejected"
        );
    }

    #[test]
    fn rf_header_encode_decode() {
        let vtime: u64 = 9_876_543_210_000;
        let size: u32 = 20;
        let rssi: i8 = -70;
        let mut hdr = [0u8; 14];
        LittleEndian::write_u64(&mut hdr[0..8], vtime);
        LittleEndian::write_u32(&mut hdr[8..12], size);
        hdr[12] = rssi as u8;
        hdr[13] = 255;
        assert_eq!(LittleEndian::read_u64(&hdr[0..8]), vtime);
        assert_eq!(LittleEndian::read_u32(&hdr[8..12]), size);
        assert_eq!(hdr[12] as i8, rssi);
    }

    #[test]
    fn rx_queue_priority_order() {
        let mut queue: Vec<(u64, usize)> = Vec::new();
        let frames = [(300u64, 30usize), (100u64, 10usize), (200u64, 20usize)];
        for (vt, sz) in frames {
            let pos = queue
                .binary_search_by(|p| p.0.cmp(&vt))
                .unwrap_or_else(|e| e);
            queue.insert(pos, (vt, sz));
        }
        assert_eq!(queue[0].0, 100);
        assert_eq!(queue[1].0, 200);
        assert_eq!(queue[2].0, 300);
    }
}
