#![allow(clippy::missing_safety_doc, clippy::collapsible_match, dead_code, unused_imports, clippy::len_zero)]

use core::ffi::{c_char, c_void};
use std::ffi::CStr;
use std::ptr;
use byteorder::{LittleEndian, ByteOrder};

use zenoh::Config;
use zenoh::Session;
use zenoh::Wait;
use zenoh::pubsub::Publisher;
use zenoh::pubsub::Subscriber;

use virtmcu_qom::sync::*;
use virtmcu_qom::timer::*;
use virtmcu_qom::irq::*;

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
    
    pan_id: u16,
    short_addr: u16,
    
    rx_timer: *mut QemuTimer,
    rx_queue: Vec<RxFrame>,
    mutex: *mut QemuMutex,
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
            eprintln!("[zenoh-802154] node={}: FAILED to open Zenoh session: {}", node_id, e);
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
    let state_ptr_raw: *mut Zenoh802154State = Box::into_raw(Box::<std::mem::MaybeUninit<Zenoh802154State>>::new_uninit()).cast();
    let state_ptr_usize = state_ptr_raw as usize;

    let subscriber = session.declare_subscriber(topic_rx)
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
        pan_id: 0xFFFF,
        short_addr: 0xFFFF,
        rx_timer,
        rx_queue: Vec::with_capacity(16),
        mutex,
    };

    ptr::write(state_ptr_raw, state);

    state_ptr_raw
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_802154_read_rust(
    state: *mut Zenoh802154State,
    offset: u64,
) -> u32 {
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
        },
        0x10 => s.rx_len,
        0x14 => s.status,
        0x18 => (s.rx_rssi as u8) as u32,
        0x20 => s.pan_id as u32,
        0x24 => s.short_addr as u32,
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
        },
        0x04 => {
            s.tx_len = (value & 0x7F) as u32;
        },
        0x08 => {
            // TX GO
            let vtime = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) as u64;
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
            qemu_set_irq(s.irq, 1);
        },
        0x14 => {
            s.status &= !(value as u32);
            if s.status & 0x01 == 0 {
                qemu_set_irq(s.irq, 0);
                let _guard = (*s.mutex).lock();
                check_rx_queue(s);
            }
        },
        0x20 => {
            s.pan_id = value as u16;
        },
        0x24 => {
            s.short_addr = value as u16;
        },
        _ => {},
    }
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_802154_cleanup_rust(state: *mut Zenoh802154State) {
    if state.is_null() { return; }
    let s = Box::from_raw(state);
    if !s.rx_timer.is_null() {
        virtmcu_timer_free(s.rx_timer);
    }
    virtmcu_mutex_free(s.mutex);
}

fn on_rx_frame(state: &mut Zenoh802154State, sample: zenoh::sample::Sample) {
    let payload = sample.payload();
    if payload.len() < 14 { return; }
    
    let bytes = payload.to_bytes();
    let vtime = LittleEndian::read_u64(&bytes[0..8]);
    let size = LittleEndian::read_u32(&bytes[8..12]) as usize;
    let rssi = bytes[12] as i8;
    
    if size > 128 || bytes.len() < 14 + size { return; }
    
    let frame_data = &bytes[14..14+size];
    
    // Slice 2: Address Filtering
    if !frame_matches_address(state, frame_data) {
        return;
    }
    
    let mut stored_data = [0u8; 128];
    stored_data[..size].copy_from_slice(frame_data);
    
    // CRITICAL: Acquire BQL before modifying QEMU timer state or taking internal locks
    // to prevent AB-BA deadlocks with the QEMU main thread.
    let _bql_guard = virtmcu_qom::sync::Bql::lock();
    let _mutex_guard = unsafe { (*state.mutex).lock() };
    
    if state.rx_queue.len() < 16 {
        // Insertion sort by vtime (ascending)
        let pos = state.rx_queue.binary_search_by(|probe| probe.delivery_vtime.cmp(&vtime))
            .unwrap_or_else(|e| e);
        state.rx_queue.insert(pos, RxFrame { delivery_vtime: vtime, data: stored_data, size, rssi });
        
        unsafe {
            virtmcu_timer_mod(state.rx_timer, state.rx_queue[0].delivery_vtime as i64);
        }
    }
}

fn frame_matches_address(s: &Zenoh802154State, frame: &[u8]) -> bool {
    if frame.len() < 3 { return false; }
    
    let fcf = LittleEndian::read_u16(&frame[0..2]);
    // let seq = frame[2];
    
    let dest_addr_mode = (fcf >> 10) & 0x03;
    
    match dest_addr_mode {
        0x00 => {
            // No destination address. Accepted if it's a beacon or if we are a coordinator.
            // For now, let's just accept it.
            true
        },
        0x02 => {
            // 16-bit short address
            if frame.len() < 7 { return false; }
            let dest_pan = LittleEndian::read_u16(&frame[3..5]);
            let dest_addr = LittleEndian::read_u16(&frame[5..7]);
            
            let pan_matches = dest_pan == 0xFFFF || dest_pan == s.pan_id;
            let addr_matches = dest_addr == 0xFFFF || dest_addr == s.short_addr;
            
            pan_matches && addr_matches
        },
        0x03 => {
            // 64-bit extended address
            if frame.len() < 13 { return false; }
            let dest_pan = LittleEndian::read_u16(&frame[3..5]);
            // For now, we only support short addresses, so we only match broadcast PAN
            // and don't match extended address unless it's broadcast (not really a thing for extended?)
            dest_pan == 0xFFFF || dest_pan == s.pan_id
        },
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
    #![deny(unsafe_code)]
    use byteorder::{LittleEndian, ByteOrder};

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
            let pos = queue.binary_search_by(|p| p.0.cmp(&vt)).unwrap_or_else(|e| e);
            queue.insert(pos, (vt, sz));
        }
        assert_eq!(queue[0].0, 100); // earliest vtime first
        assert_eq!(queue[1].0, 200);
        assert_eq!(queue[2].0, 300);
    }
}
