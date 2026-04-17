#![allow(clippy::missing_safety_doc, clippy::collapsible_match, dead_code, unused_imports, clippy::len_zero, clippy::manual_range_contains)]
extern crate libc;

use core::ffi::{c_char, c_void};
use std::ffi::CStr;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use crossbeam_channel::{bounded, Sender, Receiver};

use zenoh::Config;
use zenoh::Session;
use zenoh::Wait;
use zenoh::pubsub::Publisher;

use virtmcu_qom::sync::*;
use virtmcu_qom::timer::*;
use virtmcu_qom::cpu::*;

// We'll use FlatBufferBuilder directly to avoid version mismatch with generated code.
use flatbuffers::{FlatBufferBuilder, WIPOffset};

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
    publish_thread: Option<std::thread::JoinHandle<()>>,
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
        publish_thread: Some(thread),
        last_halted: Default::default(),
    }));

    GLOBAL_TELEMETRY = state;
    state
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_telemetry_cleanup_rust(state: *mut ZenohTelemetryState) {
    if state.is_null() { return; }
    GLOBAL_TELEMETRY = ptr::null_mut();
    
    let mut s = Box::from_raw(state);
    let _ = s.sender.send(None);
    if let Some(t) = s.publish_thread.take() {
        let _ = t.join();
    }
}

fn telemetry_worker(rx: Receiver<Option<TraceEvent>>, session: Session, topic: String) {
    let publisher: Publisher<'static> = session.declare_publisher(topic).wait().unwrap();
    let mut builder = FlatBufferBuilder::new();
    
    while let Ok(Some(ev)) = rx.recv() {
        builder.reset();
        
        let device_name_off = ev.device_name.as_deref().map(|s| builder.create_string(s));
        
        let start = builder.start_table();
        builder.push_slot(0, ev.timestamp_ns, 0); // timestamp_ns
        builder.push_slot(1, ev.event_type, 0);   // type
        builder.push_slot(2, ev.id, 0);           // id
        builder.push_slot(3, ev.value, 0);        // value
        if let Some(dn) = device_name_off {
            builder.push_slot_always(4, dn);      // device_name
        }
        let root = builder.end_table(start);
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
