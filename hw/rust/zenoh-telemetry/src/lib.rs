extern crate libc;

use core::ffi::{c_char, c_void};
use std::ffi::CStr;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use crossbeam_channel::{bounded, Sender, Receiver};

use zenoh::Config;
use zenoh::Session;
use zenoh::Wait;

use virtmcu_qom::sync::*;
use virtmcu_qom::timer::*;
use virtmcu_qom::cpu::*;

mod telemetry;
use telemetry::virtmcu::telemetry::*;

pub struct TraceEvent {
    timestamp_ns: u64,
    event_type: i8,
    id: u32,
    value: u32,
    device_name: Option<String>,
}

pub struct ZenohTelemetryState {
    #[allow(dead_code)]
    session: Session,
    sender: Sender<Option<TraceEvent>>,
    #[allow(dead_code)]
    publish_thread: std::thread::JoinHandle<()>,
    last_halted: [AtomicBool; 32],
}

static mut GLOBAL_TELEMETRY: *mut ZenohTelemetryState = ptr::null_mut();

#[no_mangle]
pub unsafe extern "C" fn zenoh_telemetry_init(
    node_id: u32,
    router: *const c_char,
) -> *mut ZenohTelemetryState {
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
            eprintln!("[zenoh-telemetry] node={}: FAILED to open Zenoh session: {}", node_id, e);
            return ptr::null_mut();
        }
    };

    let topic = format!("sim/telemetry/trace/{}", node_id);
    let (tx, rx) = bounded(1024);
    
    let sess_clone = session.clone();
    let thread = std::thread::spawn(move || {
        telemetry_worker(rx, sess_clone, topic);
    });

    let state = Box::into_raw(Box::new(ZenohTelemetryState {
        session,
        sender: tx,
        publish_thread: thread,
        last_halted: Default::default(),
    }));

    GLOBAL_TELEMETRY = state;
    state
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_telemetry_cleanup_rust(state: *mut ZenohTelemetryState) {
    if state.is_null() { return; }
    GLOBAL_TELEMETRY = ptr::null_mut();
    
    let s = Box::from_raw(state);
    let _ = s.sender.send(None); // Signal thread to exit
    // Thread joined when s is dropped
}

fn telemetry_worker(rx: Receiver<Option<TraceEvent>>, session: Session, topic: String) {
    let publisher = session.declare_publisher(topic).wait().unwrap();
    let mut builder = flatbuffers::FlatBufferBuilder::with_capacity(1024);
    
    while let Ok(Some(ev)) = rx.recv() {
        builder.reset();
        
        let device_name = ev.device_name.as_deref().map(|s| builder.create_string(s));
        
        let mut event_builder = TraceEventBuilder::new(&mut builder);
        event_builder.add_timestamp_ns(ev.timestamp_ns);
        event_builder.add_type_(TraceEventType(ev.event_type));
        event_builder.add_id(ev.id);
        event_builder.add_value(ev.value);
        if let Some(dn) = device_name {
            event_builder.add_device_name(dn);
        }
        let root = event_builder.finish();
        builder.finish(root, None);
        
        let buf = builder.finished_data();
        let _ = publisher.put(buf).wait();
    }
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_telemetry_cpu_halt_hook(cpu_index: i32, halted: bool) {
    let s = &*GLOBAL_TELEMETRY;
    if cpu_index < 0 || cpu_index >= 32 { return; }
    
    let was_halted = s.last_halted[cpu_index as usize].swap(halted, Ordering::SeqCst);
    if was_halted == halted { return; }
    
    let vtime = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL);
    let _ = s.sender.try_send(Some(TraceEvent {
        timestamp_ns: vtime as u64,
        event_type: 0, // CPU_STATE
        id: cpu_index as u32,
        value: if halted { 1 } else { 0 },
        device_name: None,
    }));
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_telemetry_irq_hook(slot: u16, pin: u16, level: i32) {
    let s = &*GLOBAL_TELEMETRY;
    let id = ((slot as u32) << 16) | (pin as u32);
    let vtime = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL);
    let _ = s.sender.try_send(Some(TraceEvent {
        timestamp_ns: vtime as u64,
        event_type: 1, // IRQ
        id,
        value: level as u32,
        device_name: None,
    }));
}
