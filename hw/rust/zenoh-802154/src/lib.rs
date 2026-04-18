#![allow(
    clippy::missing_safety_doc,
    clippy::collapsible_match,
    dead_code,
    unused_imports,
    clippy::len_zero
)]

use byteorder::{ByteOrder, LittleEndian};
use core::ffi::{c_char, c_void};
use std::ffi::CStr;
use std::ptr;

use zenoh::pubsub::Publisher;
use zenoh::pubsub::Subscriber;
use zenoh::Config;
use zenoh::Session;
use zenoh::Wait;

use virtmcu_qom::irq::*;
use virtmcu_qom::sync::*;
use virtmcu_qom::timer::*;

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

#[no_mangle]
pub unsafe extern "C" fn zenoh_802154_init_rust(
    irq: qemu_irq,
    node_id: u32,
    router: *const c_char,
    topic: *const c_char,
) -> *mut Zenoh802154State {
    let mut config = Config::default();
    if !router.is_null() {
        let router_str = CStr::from_ptr(router).to_str().unwrap();
        if !router_str.is_empty() {
            let json = format!("[\"{}\"]", router_str);
            let _ = config.insert_json5("connect/endpoints", &json);
            let _ = config.insert_json5("scouting/multicast/enabled", "false");
        }
    }

    let session = match zenoh::open(config).wait() {
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
    if !topic.is_null() {
        let t = CStr::from_ptr(topic).to_str().unwrap();
        topic_tx = format!("{}/tx", t);
        topic_rx = format!("{}/rx", t);
    } else {
        topic_tx = format!("sim/rf/802154/{}/tx", node_id);
        topic_rx = format!("sim/rf/802154/{}/rx", node_id);
    }

    let publisher = session.declare_publisher(topic_tx).wait().unwrap();

    // Two-phase init: allocate first for a stable address the subscriber captures,
    // then write the constructed state. Box::new_uninit() panics on OOM (no null UB).
    let state_ptr_raw: *mut Zenoh802154State =
        Box::into_raw(Box::<std::mem::MaybeUninit<Zenoh802154State>>::new_uninit()).cast();
    let state_ptr_usize = state_ptr_raw as usize;

    let subscriber = session
        .declare_subscriber(topic_rx)
        .callback(move |sample| {
            let state = &mut *(state_ptr_usize as *mut Zenoh802154State);
            on_rx_frame(state, sample);
        })
        .wait()
        .unwrap();

    let rx_timer = virtmcu_timer_new_ns(
        QEMU_CLOCK_VIRTUAL,
        rx_timer_cb,
        state_ptr_raw as *mut c_void,
    );

    let backoff_timer = virtmcu_timer_new_ns(
        QEMU_CLOCK_VIRTUAL,
        backoff_timer_cb,
        state_ptr_raw as *mut c_void,
    );

    let ack_timer = virtmcu_timer_new_ns(
        QEMU_CLOCK_VIRTUAL,
        ack_timer_cb,
        state_ptr_raw as *mut c_void,
    );

    let mutex = virtmcu_mutex_new();

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

    ptr::write(state_ptr_raw, state);

    state_ptr_raw
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_802154_read_rust(state: *mut Zenoh802154State, offset: u64) -> u32 {
    let s = &mut *state;
    match offset {
        0x04 => s.tx_len,
        0x0C => {
            if (s.status & 0x01 != 0) && (s.rx_read_pos < s.rx_len) {
                let val = s.rx_fifo[s.rx_read_pos as usize] as u32;
                s.rx_read_pos += 1;
                val
            } else {
                0
            }
        }
        0x10 => s.rx_len,
        0x14 => s.status | ((s.state as u32) << 8),
        0x18 => (s.rx_rssi as u8) as u32,
        0x1C => s.state as u32,
        0x20 => s.pan_id as u32,
        0x24 => s.short_addr as u32,
        0x28 => (s.ext_addr & 0xFFFFFFFF) as u32,
        0x2C => (s.ext_addr >> 32) as u32,
        _ => 0,
    }
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_802154_write_rust(
    state: *mut Zenoh802154State,
    offset: u64,
    value: u64,
) {
    assert!(!state.is_null(), "state pointer is null");
    let s = &mut *state;
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
                qemu_set_irq(s.irq, 0);
                let _guard = (*s.mutex).lock();
                check_rx_queue(s);
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
            s.ext_addr = (s.ext_addr & 0xFFFFFFFF00000000) | value;
        }
        0x2C => {
            s.ext_addr = (s.ext_addr & 0x00000000FFFFFFFF) | (value << 32);
        }
        _ => {}
    }
}

const UNIT_BACKOFF_PERIOD_NS: u64 = 320_000;
const SIFS_NS: u64 = 192_000;
const MAC_MIN_BE: u8 = 3;
const MAC_MAX_BE: u8 = 5;
const MAC_MAX_CSMA_BACKOFFS: u8 = 4;

#[no_mangle]
pub unsafe extern "C" fn zenoh_802154_cleanup_rust(state: *mut Zenoh802154State) {
    if state.is_null() {
        return;
    }
    let s = Box::from_raw(state);
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

fn tx_go(s: &mut Zenoh802154State) {
    // Slice 4: CSMA/CA Start
    s.nb = 0;
    s.be = MAC_MIN_BE;
    s.state = RadioState::Tx; // We are in TX process (BusyTx)
    schedule_backoff(s);
}

fn schedule_backoff(s: &mut Zenoh802154State) {
    // Generate random backoff between 0 and 2^BE - 1
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

    // Perform CCA
    // For now, channel is busy if we have something in RX queue that is currently "being received"
    // or if we just received something.
    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;
    let busy = !s.rx_queue.is_empty() && s.rx_queue[0].delivery_vtime <= now;

    if !busy {
        tx_real(s);
    } else {
        s.nb += 1;
        if s.nb > MAC_MAX_CSMA_BACKOFFS {
            // TX failed (Channel Busy)
            s.tx_len = 0;
            s.state = RadioState::Idle;
            // Maybe set a NO_ACK or BUSY status bit?
            // For now, just raise IRQ as if done but maybe with a different bit.
            s.status |= 0x02; // Still set TX_DONE for now to avoid hanging firmware
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

    // 802.15.4 ACK frame
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

    // Slice 2: Address Filtering
    if !frame_matches_address(state.pan_id, state.short_addr, state.ext_addr, frame_data) {
        return;
    }

    // Slice 5: Auto-ACK Request detection
    if frame_data.len() >= 3 {
        let fcf = LittleEndian::read_u16(&frame_data[0..2]);
        if (fcf & (1 << 5)) != 0 {
            // ACK requested
            state.ack_pending = true;
            state.ack_seq = frame_data[2];
            unsafe {
                virtmcu_timer_mod(state.ack_timer, (vtime + SIFS_NS) as i64);
            }
        }
    }

    let mut stored_data = [0u8; 128];
    stored_data[..size].copy_from_slice(frame_data);

    // CRITICAL: Acquire BQL before modifying QEMU timer state or taking internal locks
    // to prevent AB-BA deadlocks with the QEMU main thread.
    let _bql_guard = virtmcu_qom::sync::Bql::lock();
    let _mutex_guard = unsafe { (*state.mutex).lock() };

    if state.rx_queue.len() < 16 {
        // Insertion sort by vtime (ascending)
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
    // let seq = frame[2];

    let dest_addr_mode = (fcf >> 10) & 0x03;

    match dest_addr_mode {
        0x00 => {
            // No destination address. Accepted if it's a beacon or if we are a coordinator.
            // For now, let's just accept it.
            true
        }
        0x02 => {
            // 16-bit short address
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
            // 64-bit extended address
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

                s.status |= 0x01; // RX_READY
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

        // FCF 0x0801 (dest mode 2: short), seq 0, PAN FFFF, Addr FFFF
        let mut frame = vec![0x01, 0x08, 0x00, 0xFF, 0xFF, 0xFF, 0xFF];
        assert!(
            frame_matches_address(pan, short, ext, &frame),
            "Broadcast should be accepted"
        );

        // Broadcast PAN, match short addr
        frame[5] = 0x78;
        frame[6] = 0x56;
        assert!(
            frame_matches_address(pan, short, ext, &frame),
            "Broadcast PAN, matching short addr"
        );

        // Match PAN, Broadcast short addr
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

        // FCF 0x0801, seq 0, PAN ABCD, Addr 1234
        let frame = vec![0x01, 0x08, 0x00, 0xCD, 0xAB, 0x34, 0x12];
        assert!(
            frame_matches_address(pan, short, ext, &frame),
            "Exact match should be accepted"
        );

        // Wrong PAN
        let frame_wrong_pan = vec![0x01, 0x08, 0x00, 0x00, 0x00, 0x34, 0x12];
        assert!(
            !frame_matches_address(pan, short, ext, &frame_wrong_pan),
            "Wrong PAN should be rejected"
        );

        // Wrong Addr
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

        // FCF 0x0C01 (dest mode 3: ext), seq 0, PAN ABCD, Addr 1122334455667788
        let frame = vec![
            0x01, 0x0C, 0x00, 0xCD, 0xAB, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11,
        ];
        assert!(
            frame_matches_address(pan, short, ext, &frame),
            "Exact extended match should be accepted"
        );

        // Wrong PAN
        let frame_wrong_pan = vec![
            0x01, 0x0C, 0x00, 0x00, 0x00, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11,
        ];
        assert!(
            !frame_matches_address(pan, short, ext, &frame_wrong_pan),
            "Wrong PAN should be rejected"
        );

        // Wrong Addr
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
        hdr[13] = 255; // LQI
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
        assert_eq!(queue[0].0, 100); // earliest vtime first
        assert_eq!(queue[1].0, 200);
        assert_eq!(queue[2].0, 300);
    }
}
