//! Zenoh-based telemetry peripheral for VirtMCU simulation.

use core::ffi::{c_char, c_int, c_void};
use crossbeam_channel::{bounded, Receiver, Sender};
use flatbuffers::FlatBufferBuilder;
extern crate alloc;
use alloc::sync::Arc;
use core::ffi::CStr;
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use virtmcu_api::{telemetry_fb, TraceEvent};
use virtmcu_qom::cpu::CPUState;
use virtmcu_qom::error_setg;
use virtmcu_qom::qom::{
    object_child_foreach_recursive, object_dynamic_cast, object_get_canonical_path,
    object_get_root, Object, ObjectClass, TypeInfo, TYPE_DEVICE,
};
use virtmcu_qom::timer::{qemu_clock_get_ns, QEMU_CLOCK_VIRTUAL};
use virtmcu_qom::{declare_device_type, define_prop_string, define_prop_uint32, device_class};
use zenoh::Session;
use zenoh::Wait;

/* ── FFI Types ────────────────────────────────────────────────────────────── */

extern "C" {
    static mut virtmcu_cpu_halt_hook: Option<extern "C" fn(cpu: *mut CPUState, halted: bool)>;
    static mut virtmcu_irq_hook: Option<extern "C" fn(opaque: *mut c_void, n: c_int, level: c_int)>;
}

/* ── QOM Object ───────────────────────────────────────────────────────────── */

/// Zenoh-based telemetry device.
#[repr(C)]
pub struct ZenohTelemetryQOM {
    /// Parent object.
    pub parent_obj: virtmcu_qom::qdev::SysBusDevice,

    /* Properties */
    /// Unique node ID for telemetry.
    pub node_id: u32,
    /// Optional Zenoh router address.
    pub router: *mut c_char,

    /* Rust state */
    /// Opaque pointer to the Rust backend state.
    pub rust_state: *mut ZenohTelemetryBackend,
}

struct IrqSlot {
    opaque: *mut c_void,
    slot: u16,
    path: *mut c_char,
}

/// Internal Rust backend for `ZenohTelemetryQOM`.
pub struct ZenohTelemetryBackend {
    _session: Arc<Session>,
    sender: Sender<Option<TraceEvent>>,
    _node_id: u32,
    last_halted: Arc<[AtomicBool; 32]>,
    irq_slots: virtmcu_qom::sync::BqlGuarded<Vec<IrqSlot>>,
}

// SAFETY: ZenohTelemetryBackend encapsulates cross-thread channel sender and atomic state.
unsafe impl Send for ZenohTelemetryBackend {}
// SAFETY: ZenohTelemetryBackend's fields are internally synchronized (Atomic, Sender, BqlGuarded).
unsafe impl Sync for ZenohTelemetryBackend {}

static GLOBAL_TELEMETRY: AtomicPtr<ZenohTelemetryQOM> = AtomicPtr::new(ptr::null_mut());

extern "C" fn telemetry_cpu_halt_cb(cpu: *mut CPUState, halted: bool) {
    let s_ptr = GLOBAL_TELEMETRY.load(Ordering::Acquire);
    if s_ptr.is_null() {
        return;
    }
    // SAFETY: s_ptr is checked for null and points to the global device instance.
    let s = unsafe { &*s_ptr };
    if s.rust_state.is_null() {
        return;
    }
    // SAFETY: rust_state is non-null and cpu is a valid pointer from QEMU.
    unsafe {
        let backend = &*s.rust_state;
        zenoh_telemetry_trace_cpu_internal(backend, (*cpu).cpu_index, halted);
    }
}

extern "C" fn telemetry_irq_cb(opaque: *mut c_void, n: c_int, level: c_int) {
    let s_ptr = GLOBAL_TELEMETRY.load(Ordering::Acquire);
    if s_ptr.is_null() {
        return;
    }
    // SAFETY: s_ptr is checked for null and points to the global device instance.
    let s = unsafe { &*s_ptr };
    if s.rust_state.is_null() {
        return;
    }
    // SAFETY: rust_state is non-null.
    unsafe {
        let backend = &*s.rust_state;

        let slot_info = {
            let mut slots = backend.irq_slots.get_mut();
            let mut found_slot = None;
            for slot in slots.iter() {
                if slot.opaque == opaque {
                    found_slot = Some((slot.slot, slot.path));
                    break;
                }
            }

            if found_slot.is_none() && slots.len() < 64 {
                let new_slot = slots.len() as u16;
                slots.push(IrqSlot { opaque, slot: new_slot, path: ptr::null_mut() });
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
        let s_ptr = GLOBAL_TELEMETRY.load(Ordering::Acquire);
        if s_ptr.is_null() {
            return 0;
        }
        let s = &*s_ptr;
        let backend = &*s.rust_state;
        let mut slots = backend.irq_slots.get_mut();
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

    assert!(virtmcu_qom::sync::Bql::is_held());

    let router_ptr = if s.router.is_null() { ptr::null() } else { s.router.cast_const() };

    s.rust_state = zenoh_telemetry_init_internal(s.node_id, router_ptr);
    if s.rust_state.is_null() {
        error_setg!(errp, "zenoh-telemetry: failed to initialize Rust backend");
        return;
    }

    // SAFETY: Setting global telemetry hooks is safe here as BQL is held and initialization succeeded.
    unsafe {
        GLOBAL_TELEMETRY.store(core::ptr::from_mut::<ZenohTelemetryQOM>(s), Ordering::Release);
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

    if core::ptr::eq(s, GLOBAL_TELEMETRY.load(Ordering::Acquire)) {
        // SAFETY: Clearing global telemetry hooks is safe during finalize.
        unsafe {
            virtmcu_cpu_halt_hook = None;
            virtmcu_irq_hook = None;
            GLOBAL_TELEMETRY.store(ptr::null_mut(), Ordering::Release);
        }
    }

    if !s.rust_state.is_null() {
        let backend = Box::from_raw(s.rust_state);
        let _ = backend.sender.send(None);

        let mut slots = backend.irq_slots.get_mut();
        for _slot in slots.iter_mut() {
            // Freeing canonical path would require calling QEMU free
        }
        s.rust_state = ptr::null_mut();
    }
}

/* ── Properties ───────────────────────────────────────────────────────────── */

static ZENOH_TELEMETRY_PROPERTIES: [virtmcu_qom::qom::Property; 3] = [
    define_prop_uint32!(c"node".as_ptr(), ZenohTelemetryQOM, node_id, 0),
    define_prop_string!(c"router".as_ptr(), ZenohTelemetryQOM, router),
    // SAFETY: QEMU expects a zeroed Property as a sentinel.
    unsafe { core::mem::zeroed() },
];

/* ── Class Init ───────────────────────────────────────────────────────────── */

unsafe extern "C" fn zenoh_telemetry_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let dc = device_class!(klass);
    (*dc).realize = Some(zenoh_telemetry_realize);
    virtmcu_qom::qdev::device_class_set_props_n(dc, ZENOH_TELEMETRY_PROPERTIES.as_ptr(), 2);
}

static ZENOH_TELEMETRY_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"zenoh-telemetry".as_ptr(),
    parent: c"sys-bus-device".as_ptr(),
    instance_size: core::mem::size_of::<ZenohTelemetryQOM>(),
    instance_align: 0,
    instance_init: None,
    instance_post_init: None,
    instance_finalize: Some(zenoh_telemetry_instance_finalize),
    abstract_: false,
    class_size: 0,
    class_init: Some(zenoh_telemetry_class_init),
    class_base_init: None,
    class_data: core::ptr::null(),
    interfaces: core::ptr::null(),
};

declare_device_type!(ZENOH_TELEMETRY_TYPE_INIT, ZENOH_TELEMETRY_TYPE_INFO);

/* ── Internal Logic ───────────────────────────────────────────────────────── */

fn zenoh_telemetry_init_internal(
    node_id: u32,
    router: *const c_char,
) -> *mut ZenohTelemetryBackend {
    // SAFETY: router pointer is valid or null.
    let session = unsafe {
        match virtmcu_zenoh::get_or_init_session(router) {
            Ok(s) => s,
            Err(e) => {
                virtmcu_qom::vlog!(
                    "[zenoh-telemetry] node={node_id}: FAILED to open Zenoh session: {e}"
                );
                return ptr::null_mut();
            }
        }
    };

    let (tx, rx) = bounded(1024);
    let topic = format!("sim/telemetry/trace/{node_id}");
    let sess_clone = Arc::clone(&session);

    std::thread::spawn(move || {
        telemetry_worker(rx, sess_clone, topic);
    });

    Box::into_raw(Box::new(ZenohTelemetryBackend {
        _session: session,
        sender: tx,
        _node_id: node_id,
        last_halted: Arc::new(core::array::from_fn(|_| AtomicBool::new(false))),
        irq_slots: virtmcu_qom::sync::BqlGuarded::new(Vec::with_capacity(64)),
    }))
}

fn zenoh_telemetry_trace_cpu_internal(
    backend: &ZenohTelemetryBackend,
    cpu_index: i32,
    halted: bool,
) {
    if !(0..32).contains(&cpu_index) {
        return;
    }

    let was_halted = backend.last_halted[cpu_index as usize].swap(halted, Ordering::SeqCst);
    if was_halted == halted {
        return;
    }

    let vtime =
        // SAFETY: Calling qemu_clock_get_ns is safe within BQL context or vCPU thread.
        unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };
    let _ = backend.sender.try_send(Some(TraceEvent {
        timestamp_ns: vtime as u64,
        event_type: 0,
        id: cpu_index as u32,
        value: u32::from(halted),
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
    let id = (u32::from(slot) << 16) | u32::from(pin);
    let vtime =
        // SAFETY: Calling qemu_clock_get_ns is safe within BQL context or vCPU thread.
        unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };
    let device_name = if name_ptr.is_null() {
        None
    } else {
        // SAFETY: name_ptr is a valid null-terminated C string provided by QEMU.
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

fn telemetry_worker(rx: Receiver<Option<TraceEvent>>, session: Arc<Session>, topic: String) {
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

    #[test]
    fn test_zenoh_telemetry_qom_layout() {
        // QOM layout validation
        assert_eq!(
            core::mem::offset_of!(ZenohTelemetryQOM, parent_obj),
            0,
            "SysBusDevice must be the first field"
        );
    }
}
