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
    clippy::manual_range_contains
)]
extern crate libc;

use core::ffi::{c_char, c_int, c_void};
use crossbeam_channel::{bounded, Receiver, Sender};
use flatbuffers::{FlatBufferBuilder, WIPOffset};
use std::ffi::{CStr, CString};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use virtmcu_api::{telemetry_fb, TraceEvent};
use virtmcu_qom::cpu::{virtmcu_cpu_halt_hook, CPUState};
use virtmcu_qom::error::Error;
use virtmcu_qom::irq::virtmcu_irq_hook;
use virtmcu_qom::qdev::{DeviceClass, SysBusDevice};
use virtmcu_qom::qom::{
    object_child_foreach_recursive, object_dynamic_cast, object_get_canonical_path,
    object_get_root, Object, ObjectClass, TypeInfo, TYPE_DEVICE,
};
use virtmcu_qom::sync::virtmcu_bql_locked;
use virtmcu_qom::timer::{qemu_clock_get_ns, QEMU_CLOCK_VIRTUAL};
use virtmcu_qom::{
    declare_device_type, define_prop_string, define_prop_uint32, define_properties, device_class,
    error_setg,
};
use zenoh::{Config, Session, Wait};

#[repr(C)]
pub struct ZenohTelemetryQOM {
    pub parent_obj: SysBusDevice,
    pub node_id: u32,
    pub router: *mut c_char,
    pub rust_state: *mut ZenohTelemetryBackend,
}

struct IrqSlot {
    opaque: *mut c_void,
    slot: u16,
    path: *mut c_char,
}

pub struct ZenohTelemetryBackend {
    session: Session,
    sender: Sender<Option<TraceEvent>>,
    node_id: u32,
    last_halted: Arc<[AtomicBool; 32]>,
    irq_slots: Mutex<Vec<IrqSlot>>,
}

unsafe impl Send for ZenohTelemetryBackend {}
unsafe impl Sync for ZenohTelemetryBackend {}

static mut GLOBAL_TELEMETRY: *mut ZenohTelemetryQOM = ptr::null_mut();

extern "C" fn telemetry_cpu_halt_cb(cpu: *mut CPUState, halted: bool) {
    let s = unsafe { &*GLOBAL_TELEMETRY };
    if s.rust_state.is_null() {
        return;
    }
    unsafe {
        let backend = &*s.rust_state;
        zenoh_telemetry_trace_cpu_internal(backend, (*cpu).cpu_index, halted);
    }
}

extern "C" fn telemetry_irq_cb(opaque: *mut c_void, n: c_int, level: c_int) {
    let s = unsafe { &*GLOBAL_TELEMETRY };
    if s.rust_state.is_null() {
        return;
    }
    unsafe {
        let backend = &*s.rust_state;

        let slot_info = {
            let mut slots = backend.irq_slots.lock().unwrap();
            let mut found_slot = None;
            for slot in slots.iter() {
                if slot.opaque == opaque {
                    found_slot = Some((slot.slot, slot.path));
                    break;
                }
            }

            if found_slot.is_none() && slots.len() < 64 {
                let new_slot = slots.len() as u16;
                slots.push(IrqSlot {
                    opaque,
                    slot: new_slot,
                    path: ptr::null_mut(),
                });
                found_slot = Some((new_slot, ptr::null_mut()));
            }
            found_slot
        };

        if let Some((slot, path)) = slot_info {
            zenoh_telemetry_trace_irq_internal(backend, slot, n as u16, level, path);
        }
    }
}

unsafe extern "C" fn cache_irq_paths_cb(obj: *mut Object, _opaque: *mut c_void) -> c_int {
    if !object_dynamic_cast(obj, TYPE_DEVICE).is_null() {
        let s = &*GLOBAL_TELEMETRY;
        let backend = &*s.rust_state;
        let mut slots = backend.irq_slots.lock().unwrap();
        let len = slots.len();
        if len < 64 {
            slots.push(IrqSlot {
                opaque: obj as *mut c_void,
                slot: len as u16,
                path: object_get_canonical_path(obj),
            });
        }
    }
    0
}

unsafe extern "C" fn zenoh_telemetry_realize(dev: *mut c_void, errp: *mut *mut c_void) {
    let s = &mut *(dev as *mut ZenohTelemetryQOM);

    assert!(virtmcu_bql_locked());

    let router_ptr = if s.router.is_null() {
        ptr::null()
    } else {
        s.router as *const c_char
    };

    s.rust_state = zenoh_telemetry_init_internal(s.node_id, router_ptr);
    if s.rust_state.is_null() {
        error_setg!(
            errp as *mut *mut Error,
            c"zenoh-telemetry: failed to initialize Rust backend".as_ptr()
        );
        return;
    }

    unsafe {
        GLOBAL_TELEMETRY = s;
        object_child_foreach_recursive(
            object_get_root(),
            Some(cache_irq_paths_cb),
            ptr::null_mut(),
        );
        virtmcu_cpu_halt_hook = Some(telemetry_cpu_halt_cb);
        virtmcu_irq_hook = Some(telemetry_irq_cb);
    }
}

unsafe extern "C" fn zenoh_telemetry_instance_finalize(obj: *mut Object) {
    let s = &mut *(obj as *mut ZenohTelemetryQOM);

    if s as *mut ZenohTelemetryQOM == GLOBAL_TELEMETRY {
        unsafe {
            virtmcu_cpu_halt_hook = None;
            virtmcu_irq_hook = None;
            GLOBAL_TELEMETRY = ptr::null_mut();
        }
    }

    if !s.rust_state.is_null() {
        let backend = Box::from_raw(s.rust_state);
        let _ = backend.sender.send(None);

        let slots = backend.irq_slots.lock().unwrap();
        for slot in slots.iter() {
            if !slot.path.is_null() {
                libc::free(slot.path as *mut c_void);
            }
        }
        s.rust_state = ptr::null_mut();
    }
}

define_properties!(
    ZENOH_TELEMETRY_PROPERTIES,
    [
        define_prop_uint32!(c"node".as_ptr(), ZenohTelemetryQOM, node_id, 0),
        define_prop_string!(c"router".as_ptr(), ZenohTelemetryQOM, router),
    ]
);

unsafe extern "C" fn zenoh_telemetry_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let dc = device_class!(klass);
    unsafe {
        (*dc).realize = Some(zenoh_telemetry_realize);
        (*dc).user_creatable = true;
    }
    virtmcu_qom::device_class_set_props!(dc, ZENOH_TELEMETRY_PROPERTIES);
}

static ZENOH_TELEMETRY_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"zenoh-telemetry".as_ptr(),
    parent: c"sys-bus-device".as_ptr(),
    instance_size: std::mem::size_of::<ZenohTelemetryQOM>(),
    instance_align: 0,
    instance_init: None,
    instance_post_init: None,
    instance_finalize: Some(zenoh_telemetry_instance_finalize),
    abstract_: false,
    class_size: 0,
    class_init: Some(zenoh_telemetry_class_init),
    class_base_init: None,
    class_data: ptr::null(),
    interfaces: ptr::null(),
};

declare_device_type!(zenoh_telemetry_type_init, ZENOH_TELEMETRY_TYPE_INFO);

/* ── Internal Logic ───────────────────────────────────────────────────────── */

fn zenoh_telemetry_init_internal(
    node_id: u32,
    router: *const c_char,
) -> *mut ZenohTelemetryBackend {
    let session = unsafe {
        match virtmcu_zenoh::open_session(router) {
            Ok(s) => s,
            Err(e) => {
                eprintln!(
                    "[zenoh-telemetry] node={}: FAILED to open Zenoh session: {}",
                    node_id, e
                );
                return ptr::null_mut();
            }
        }
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
        irq_slots: Mutex::new(Vec::with_capacity(64)),
    }))
}

fn zenoh_telemetry_trace_cpu_internal(
    backend: &ZenohTelemetryBackend,
    cpu_index: i32,
    halted: bool,
) {
    if cpu_index < 0 || cpu_index >= 32 {
        return;
    }

    let was_halted = backend.last_halted[cpu_index as usize].swap(halted, Ordering::SeqCst);
    if was_halted == halted {
        return;
    }

    let vtime = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };
    let _ = backend.sender.try_send(Some(TraceEvent {
        timestamp_ns: vtime as u64,
        event_type: 0,
        id: cpu_index as u32,
        value: if halted { 1 } else { 0 },
        device_name: None,
    }));
}

fn zenoh_telemetry_trace_irq_internal(
    backend: &ZenohTelemetryBackend,
    slot: u16,
    pin: u16,
    level: i32,
    name_ptr: *const c_char,
) {
    let id = ((slot as u32) << 16) | (pin as u32);
    let vtime = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };

    let device_name = if name_ptr.is_null() {
        None
    } else {
        unsafe { Some(CStr::from_ptr(name_ptr).to_string_lossy().into_owned()) }
    };

    let _ = backend.sender.try_send(Some(TraceEvent {
        timestamp_ns: vtime as u64,
        event_type: 1,
        id,
        value: level as u32,
        device_name,
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
// force rebuild
