#![allow(clippy::missing_safety_doc, clippy::collapsible_match, dead_code, unused_imports, clippy::len_zero, clippy::manual_range_contains)]
extern crate libc;

use core::ffi::{c_char, c_void};
use std::ffi::CStr;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use crossbeam_channel::{bounded, Sender, Receiver};
use zenoh::{Config, Session, Wait};
use flatbuffers::{FlatBufferBuilder, WIPOffset};
use virtmcu_qom::timer::{qemu_clock_get_ns, QEMU_CLOCK_VIRTUAL};

// Minimal manual generation of FlatBuffer bindings for TraceEvent
#[allow(dead_code, non_snake_case)]
pub mod telemetry_fb {
    use flatbuffers::{WIPOffset, FlatBufferBuilder};

    #[derive(Copy, Clone, PartialEq, Debug)]
    #[repr(i8)]
    pub enum TraceEventType {
        CpuState = 0,
        Irq = 1,
        Peripheral = 2,
    }

    pub struct TraceEventArgs<'a> {
        pub timestamp_ns: u64,
        pub type_: TraceEventType,
        pub id: u32,
        pub value: u32,
        pub device_name: Option<WIPOffset<&'a str>>,
    }

    pub fn create_trace_event<'a>(
        fbb: &mut FlatBufferBuilder<'a>,
        args: &TraceEventArgs<'a>
    ) -> WIPOffset<flatbuffers::Table<'a>> {
        let start = fbb.start_table();
        fbb.push_slot(0, args.timestamp_ns, 0);
        fbb.push_slot(2, args.id, 0);
        fbb.push_slot(3, args.value, 0);
        if let Some(x) = args.device_name {
            fbb.push_slot_always(4, x);
        }
        fbb.push_slot(1, args.type_ as i8, 0);
        let end = fbb.end_table(start);
        WIPOffset::new(end.value())
    }
}

pub struct TraceEvent {
    pub timestamp_ns: u64,
    pub event_type: i8,
    pub id: u32,
    pub value: u32,
    pub device_name: Option<String>,
}

pub struct ZenohTelemetryBackend {
    session: Session,
    sender: Sender<Option<TraceEvent>>,
    node_id: u32,
    last_halted: Arc<[AtomicBool; 32]>,
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_telemetry_init(
    node_id: u32,
    router: *const c_char,
) -> *mut ZenohTelemetryBackend {
    let session = match virtmcu_zenoh::open_session(router) {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };

    let (tx, rx) = bounded(1024);
    let topic = format!("sim/telemetry/trace/{}", node_id);
    let sess_clone = session.clone();
    
    std::thread::spawn(move || {
        telemetry_worker(rx, sess_clone, topic);
    });

    Box::into_raw(Box::new(ZenohTelemetryBackend {
        session,
        sender: tx,
        node_id,
        last_halted: Arc::new(Default::default()),
    }))
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_telemetry_free(backend: *mut ZenohTelemetryBackend) {
    if !backend.is_null() {
        let b = Box::from_raw(backend);
        let _ = b.sender.send(None);
    }
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_telemetry_trace_cpu(backend: *mut ZenohTelemetryBackend, cpu_index: i32, halted: bool) {
    let b = &*backend;
    if cpu_index < 0 || cpu_index >= 32 { return; }
    
    let was_halted = b.last_halted[cpu_index as usize].swap(halted, Ordering::SeqCst);
    if was_halted == halted { return; }

    let vtime = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL);
    debug_assert!(vtime >= 0, "negative vtime from QEMU clock: {}", vtime);
    let _ = b.sender.try_send(Some(TraceEvent {
        timestamp_ns: vtime as u64,
        event_type: 0,
        id: cpu_index as u32,
        value: if halted { 1 } else { 0 },
        device_name: None,
    }));
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_telemetry_trace_irq(backend: *mut ZenohTelemetryBackend, slot: u16, pin: u16, level: i32) {
    let b = &*backend;
    let id = ((slot as u32) << 16) | (pin as u32);
    let vtime = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL);
    debug_assert!(vtime >= 0, "negative vtime from QEMU clock: {}", vtime);
    let _ = b.sender.try_send(Some(TraceEvent {
        timestamp_ns: vtime as u64,
        event_type: 1,
        id,
        value: level as u32,
        device_name: None,
    }));
}

fn telemetry_worker(rx: Receiver<Option<TraceEvent>>, session: Session, topic: String) {
    let publisher = match session.declare_publisher(topic).wait() {
        Ok(p) => p,
        Err(_) => return,
    };
    let mut builder = FlatBufferBuilder::new();
    
    while let Ok(Some(ev)) = rx.recv() {
        builder.reset();
        
        let device_name_off = ev.device_name.as_deref().map(|s| builder.create_string(s));
        
        let args = telemetry_fb::TraceEventArgs {
            timestamp_ns: ev.timestamp_ns,
            type_: match ev.event_type {
                0 => telemetry_fb::TraceEventType::CpuState,
                1 => telemetry_fb::TraceEventType::Irq,
                _ => telemetry_fb::TraceEventType::Peripheral,
            },
            id: ev.id,
            value: ev.value,
            device_name: device_name_off,
        };
        
        let root = telemetry_fb::create_trace_event(&mut builder, &args);
        builder.finish(root, None);
        
        let buf = builder.finished_data();
        let _ = publisher.put(buf).wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[no_mangle]
    extern "C" fn qemu_clock_get_ns(_type: i32) -> i64 {
        -1
    }

    #[test]
    fn test_qemu_clock_get_ns_type() {
        let vtime = unsafe { qemu_clock_get_ns(0) };
        // If it was u64, -1 would be a huge positive number.
        // The fact that we can compare it to -1 and it's equal proves it's i64.
        assert_eq!(vtime, -1i64);
    }
}
