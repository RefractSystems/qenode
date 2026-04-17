extern crate libc;

use core::ffi::{c_char, c_void};
use std::ffi::CStr;
use std::ptr;
use byteorder::{LittleEndian, ByteOrder};

use zenoh::Config;
use zenoh::Session;
use zenoh::Wait;
use zenoh::pubsub::Publisher;
use zenoh::pubsub::Subscriber;

use virtmcu_qom::net::*;
use virtmcu_qom::sync::*;
use virtmcu_qom::timer::*;

#[repr(C)]
#[derive(Copy, Clone)]
struct ZenohFrameHeader {
    delivery_vtime_ns: u64,
    size: u32,
}

struct RxFrame {
    delivery_vtime: u64,
    data: Vec<u8>,
}

pub struct ZenohNetdevState {
    nc: *mut NetClientState,
    #[allow(dead_code)]
    session: Session,
    publisher: Publisher<'static>,
    #[allow(dead_code)]
    subscriber: Subscriber<()>,
    rx_timer: *mut QemuTimer,
    #[allow(dead_code)]
    node_id: u32,
    
    #[allow(dead_code)]
    mutex: *mut QemuMutex,
    rx_queue: std::sync::Mutex<Vec<RxFrame>>,
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_netdev_init(
    nc: *mut NetClientState,
    node_id: u32,
    router: *const c_char,
    topic: *const c_char,
) -> *mut ZenohNetdevState {
    let mut config = Config::default();
    if !router.is_null() {
        let router_str = unsafe { CStr::from_ptr(router) }.to_str().unwrap();
        let json = format!("[\"{}\"]", router_str);
        let _ = config.insert_json5("connect/endpoints", &json);
        let _ = config.insert_json5("scouting/multicast/enabled", "false");
    }

    let session = match zenoh::open(config).wait() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[zenoh-netdev] node={}: FAILED to open Zenoh session: {}", node_id, e);
            return ptr::null_mut();
        }
    };

    let topic_tx;
    let topic_rx;
    if !topic.is_null() {
        let t = unsafe { CStr::from_ptr(topic) }.to_str().unwrap();
        topic_tx = format!("{}/tx", t);
        topic_rx = format!("{}/rx", t);
    } else {
        topic_tx = format!("sim/eth/frame/{}/tx", node_id);
        topic_rx = format!("sim/eth/frame/{}/rx", node_id);
    }

    let publisher = session.declare_publisher(topic_tx).wait().unwrap();
    
    let mutex = unsafe { virtmcu_mutex_new() };
    
    let state_ptr_raw = unsafe { libc::malloc(std::mem::size_of::<ZenohNetdevState>()) as *mut ZenohNetdevState };
    let state_ptr_usize = state_ptr_raw as usize;

    let subscriber = session.declare_subscriber(topic_rx)
        .callback(move |sample| {
            let state = unsafe { &*(state_ptr_usize as *const ZenohNetdevState) };
            on_rx_frame(state, sample);
        })
        .wait()
        .unwrap();

    let rx_timer = unsafe { virtmcu_timer_new_ns(
        QEMU_CLOCK_VIRTUAL,
        rx_timer_cb,
        state_ptr_raw as *mut c_void,
    ) };

    let state = ZenohNetdevState {
        nc,
        session,
        publisher,
        subscriber,
        rx_timer,
        node_id,
        mutex,
        rx_queue: std::sync::Mutex::new(Vec::with_capacity(64)),
    };

    unsafe {
        ptr::write(state_ptr_raw, state);
    }

    state_ptr_raw
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_netdev_receive_rust(
    state: *mut ZenohNetdevState,
    buf: *const u8,
    size: usize,
) -> isize {
    let s = &*state;
    let vtime = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };
    
    let mut msg = Vec::with_capacity(12 + size);
    
    let mut hdr_bytes = [0u8; 12];
    LittleEndian::write_u64(&mut hdr_bytes[0..8], vtime as u64);
    LittleEndian::write_u32(&mut hdr_bytes[8..12], size as u32);
    
    msg.extend_from_slice(&hdr_bytes);
    msg.extend_from_slice(unsafe { std::slice::from_raw_parts(buf, size) });
    
    let _ = s.publisher.put(msg).wait();
    
    size as isize
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_netdev_cleanup_rust(state: *mut ZenohNetdevState) {
    if state.is_null() { return; }
    let s = unsafe { Box::from_raw(state) };
    if !s.rx_timer.is_null() {
        unsafe { virtmcu_timer_free(s.rx_timer); }
    }
    unsafe { virtmcu_mutex_free(s.mutex); }
}

fn on_rx_frame(state: &ZenohNetdevState, sample: zenoh::sample::Sample) {
    let payload = sample.payload();
    if payload.len() < 12 { return; }
    
    let bytes = payload.to_bytes();
    let vtime = LittleEndian::read_u64(&bytes[0..8]);
    let size = LittleEndian::read_u32(&bytes[8..12]) as usize;
    
    if size > 65536 || bytes.len() < 12 + size { return; }
    
    let frame_data = bytes[12..12+size].to_vec();
    
    let mut queue = state.rx_queue.lock().unwrap();
    if queue.len() < 64 {
        // Insertion sort by vtime (ascending)
        let pos = queue.binary_search_by(|probe| probe.delivery_vtime.cmp(&vtime))
            .unwrap_or_else(|e| e);
        queue.insert(pos, RxFrame { delivery_vtime: vtime, data: frame_data });
        
        // Mod timer for the earliest frame
        unsafe {
            virtmcu_timer_mod(state.rx_timer, queue[0].delivery_vtime as i64);
        }
    }
}

extern "C" fn rx_timer_cb(opaque: *mut c_void) {
    let state = unsafe { &*(opaque as *mut ZenohNetdevState) };
    
    loop {
        let frame = {
            let mut queue = state.rx_queue.lock().unwrap();
            let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;
            
            if queue.is_empty() { return; }
            
            if queue[0].delivery_vtime <= now {
                queue.remove(0)
            } else {
                unsafe {
                    virtmcu_timer_mod(state.rx_timer, queue[0].delivery_vtime as i64);
                }
                return;
            }
        };
        
        unsafe {
            qemu_send_packet(state.nc, frame.data.as_ptr(), frame.data.len());
        }
    }
}
