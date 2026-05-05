//! Virtmcu telemetry peripheral with pluggable transport.

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
use virtmcu_qom::qdev::TYPE_SYS_BUS_DEVICE;
use virtmcu_qom::qom::{
    object_child_foreach_recursive, object_dynamic_cast, object_get_canonical_path,
    object_get_root, Object, ObjectClass, TypeInfo,
};
use virtmcu_qom::timer::{qemu_clock_get_ns, QEMU_CLOCK_VIRTUAL};
use virtmcu_qom::{
    declare_device_type, define_prop_string, define_prop_uint32, define_properties, device_class,
    device_class_set_props,
};
use zenoh::Wait;

fn create_transport(
    transport_name: &str,
    router: *const c_char,
) -> Result<Arc<dyn virtmcu_api::DataTransport>, String> {
    if transport_name == "unix" {
        let path = "/tmp/virtmcu_telemetry.sock";
        match transport_unix::UnixDataTransport::new(path) {
            Ok(t) => Ok(Arc::new(t)),
            Err(e) => Err(e),
        }
    } else {
        match unsafe { transport_zenoh::get_or_init_session(router) } {
            Ok(session) => Ok(Arc::new(transport_zenoh::ZenohDataTransport::new(session))),
            Err(e) => Err(format!("FAILED to open Zenoh session: {e}")),
        }
    }
}

/* ── QOM Object ───────────────────────────────────────────────────────────── */

/// Virtmcu telemetry device.
#[repr(C)]
pub struct VirtmcuTelemetryQOM {
    /// Parent object.
    pub parent_obj: virtmcu_qom::qdev::SysBusDevice,

    /* Properties */
    /// Unique node ID for telemetry.
    pub node_id: u32,
    /// The transport to use (zenoh or unix).
    pub transport: *mut c_char,
    /// Optional Zenoh router address.
    pub router: *mut c_char,
    /// Debug flag
    pub debug: bool,

    /* Rust state */
    /// Opaque pointer to the Rust backend state.
    pub rust_state: *mut VirtmcuTelemetryBackend,
}

struct IrqSlot {
    #[allow(dead_code)] // ALLOW_EXCEPTION: Reserved for future GPIO support
    opaque: *mut c_void,
    #[allow(dead_code)] // ALLOW_EXCEPTION: Reserved for future GPIO support
    slot: u16,
    path: *mut c_char,
}

impl Drop for IrqSlot {
    fn drop(&mut self) {
        if !self.path.is_null() {
            unsafe {
                virtmcu_qom::qom::g_free(self.path as *mut c_void);
            }
        }
    }
}

/// Internal Rust backend for `VirtmcuTelemetryQOM`.
pub struct VirtmcuTelemetryBackend {
    _transport: Arc<dyn virtmcu_api::DataTransport>,
    sender: Sender<Option<TraceEvent>>,
    _node_id: u32,
    last_halted: Arc<[AtomicBool; 32]>,
    irq_slots: virtmcu_qom::sync::BqlGuarded<Vec<IrqSlot>>,
    _liveliness: Option<zenoh::liveliness::LivelinessToken>,
}

// SAFETY: VirtmcuTelemetryBackend encapsulates cross-thread channel sender and atomic state.
unsafe impl Send for VirtmcuTelemetryBackend {}
// SAFETY: VirtmcuTelemetryBackend's fields are internally synchronized (Atomic, Sender, BqlGuarded).
unsafe impl Sync for VirtmcuTelemetryBackend {}

static GLOBAL_TELEMETRY: AtomicPtr<VirtmcuTelemetryQOM> = AtomicPtr::new(ptr::null_mut());

extern "C" fn telemetry_cpu_halt_cb(cpu: *mut CPUState, halted: bool) {
    let s_ptr = GLOBAL_TELEMETRY.load(Ordering::Acquire);
    if s_ptr.is_null() {
        return;
    }
    let s = unsafe { &*s_ptr };
    if s.rust_state.is_null() {
        return;
    }
    unsafe {
        let backend = &*s.rust_state;
        telemetry_trace_cpu_internal(backend, (*cpu).cpu_index, halted);
    }
}

/*
extern "C" fn telemetry_irq_cb(opaque: *mut c_void, n: c_int, level: c_int) {
    let s_ptr = GLOBAL_TELEMETRY.load(Ordering::Acquire);
    if s_ptr.is_null() {
        return;
    }
    let s = unsafe { &*s_ptr };
    if s.rust_state.is_null() {
        return;
    }
    unsafe {
        let backend = &*s.rust_state;

        let slot_info = {
            let slots = backend.irq_slots.get();
            let mut found_slot = None;
            for slot in slots.iter() {
                if slot.opaque == opaque {
                    found_slot = Some((slot.slot, slot.path));
                    break;
                }
            }
            found_slot
        };

        if let Some((slot, name_ptr)) = slot_info {
            telemetry_trace_irq_internal(backend, slot, n as u16, level, name_ptr);
        }
    }
}
*/

fn telemetry_trace_cpu_internal(backend: &VirtmcuTelemetryBackend, cpu_index: c_int, halted: bool) {
    if !(0..32).contains(&cpu_index) {
        return;
    }

    // Only trace if state actually changed
    let prev = backend.last_halted[cpu_index as usize].swap(halted, Ordering::SeqCst);
    if prev == halted {
        return;
    }

    let vtime = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };

    let _ = backend.sender.try_send(Some(TraceEvent {
        timestamp_ns: vtime as u64,
        event_type: 0,
        id: cpu_index as u32,
        value: u32::from(halted),
        device_name: None,
        power_uw: 0,
    }));
}

/*
fn telemetry_trace_irq_internal(
    backend: &VirtmcuTelemetryBackend,
    slot: u16,
    pin: u16,
    level: i32,
    name_ptr: *const c_char,
) {
    let id = (u32::from(slot) << 16) | u32::from(pin);
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
        power_uw: 0,
    }));
}
*/

fn telemetry_worker(
    rx: Receiver<Option<TraceEvent>>,
    transport: Arc<dyn virtmcu_api::DataTransport>,
    topic: String,
) {
    let mut builder = FlatBufferBuilder::new();

    while let Ok(Some(ev)) = rx.recv() {
        builder.reset();

        let device_name_off = ev.device_name.as_deref().map(|s| builder.create_string(s));

        let args = telemetry_fb::TraceEventArgs {
            timestamp_ns: ev.timestamp_ns,
            type_: match ev.event_type {
                0 => telemetry_fb::TraceEventType::CpuState,
                1 => telemetry_fb::TraceEventType::Irq,
                3 => telemetry_fb::TraceEventType::PowerState,
                _ => telemetry_fb::TraceEventType::Peripheral,
            },
            id: ev.id,
            value: ev.value,
            device_name: device_name_off,
            power_uw: ev.power_uw,
        };

        let root = telemetry_fb::create_trace_event(&mut builder, &args);
        builder.finish(root, None);

        let payload = builder.finished_data();
        let _ = transport.publish(&topic, payload);
    }
}

/* ── QOM Methods ──────────────────────────────────────────────────────────── */

unsafe extern "C" fn telemetry_init(obj: *mut Object) {
    let s = &mut *(obj as *mut VirtmcuTelemetryQOM);
    s.node_id = 0;
    s.transport = ptr::null_mut();
    s.router = ptr::null_mut();
    s.debug = false;
    s.rust_state = ptr::null_mut();
}

unsafe extern "C" fn telemetry_realize(dev_state: *mut c_void, _errp: *mut *mut c_void) {
    let dev = dev_state as *mut virtmcu_qom::qdev::DeviceState;
    let s = &mut *(dev as *mut VirtmcuTelemetryQOM);

    let (tx, rx) = bounded(10000);

    let transport_name = if s.transport.is_null() {
        "zenoh".to_owned()
    } else {
        CStr::from_ptr(s.transport).to_string_lossy().into_owned()
    };

    let router_ptr = if s.router.is_null() { ptr::null() } else { s.router.cast_const() };
    let transport = match create_transport(&transport_name, router_ptr) {
        Ok(t) => t,
        Err(e) => {
            error_setg!(_errp, "Failed to create transport: {}", e);
            return;
        }
    };

    let node_id = s.node_id;
    let topic = format!("sim/telemetry/trace/{node_id}");

    let liveliness = if transport_name == "zenoh" {
        match transport_zenoh::get_or_init_session(router_ptr) {
            Ok(session) => {
                let key = format!("sim/telemetry/liveliness/{node_id}");
                session.liveliness().declare_token(key).wait().ok()
            }
            Err(_) => None,
        }
    } else {
        None
    };

    let backend = Box::new(VirtmcuTelemetryBackend {
        _transport: Arc::clone(&transport),
        sender: tx,
        _node_id: node_id,
        last_halted: Arc::new(Default::default()),
        irq_slots: virtmcu_qom::sync::BqlGuarded::new(Vec::new()),
        _liveliness: liveliness,
    });

    s.rust_state = Box::into_raw(backend);

    GLOBAL_TELEMETRY.store(s, Ordering::Release);

    std::thread::spawn(move || telemetry_worker(rx, transport, topic));

    virtmcu_qom::cpu::virtmcu_cpu_set_halt_hook(Some(telemetry_cpu_halt_cb));

    // Discover all GPIO devices and attach interceptors
    let root = object_get_root();
    object_child_foreach_recursive(root, Some(telemetry_gpio_discover_cb), ptr::from_mut(s).cast());
}

unsafe extern "C" fn telemetry_gpio_discover_cb(obj: *mut Object, opaque: *mut c_void) -> c_int {
    let s = &mut *(opaque as *mut VirtmcuTelemetryQOM);
    let dev = object_dynamic_cast(obj, virtmcu_qom::qdev::TYPE_DEVICE);
    if dev.is_null() {
        return 0;
    }

    let dev = dev as *mut virtmcu_qom::qdev::DeviceState;
    let backend = (&*s.rust_state).irq_slots.get();

    let path_ptr = object_get_canonical_path(obj);
    let path = if path_ptr.is_null() { ptr::null_mut() } else { path_ptr };

    let slot = backend.len() as u16;

    // FIXME: num_gpio_out is missing from DeviceState in the current virtmcu-qom bindings.
    // For now, we skip individual GPIO interception until the bindings are updated.
    /*
    for n in 0..(*dev).num_gpio_out {
        let irq = virtmcu_qom::qdev::qdev_get_gpio_out_connector(dev, n);
        if irq.is_null() {
            continue;
        }

        virtmcu_qom::qdev::qemu_irq_intercept_in(
            irq,
            Some(telemetry_irq_cb),
            s as *mut _ as *mut c_void,
        );

        backend.push(IrqSlot { opaque: s as *mut _ as *mut c_void, slot, path });
    }
    */
    let _ = (dev, slot, path, backend);

    0
}

unsafe extern "C" fn telemetry_unrealize(dev_state: *mut c_void) {
    let dev = dev_state as *mut virtmcu_qom::qdev::DeviceState;
    let s = &mut *(dev as *mut VirtmcuTelemetryQOM);
    if !s.rust_state.is_null() {
        let backend = Box::from_raw(s.rust_state);
        let _ = backend.sender.send(None); // Signal worker to stop
    }
}

unsafe extern "C" fn telemetry_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let dc = device_class!(klass);
    unsafe {
        (*dc).realize = Some(telemetry_realize);
        (*dc).unrealize = Some(telemetry_unrealize);
    }

    device_class_set_props!(dc, TELEMETRY_PROPERTIES);
}

define_properties!(
    TELEMETRY_PROPERTIES,
    [
        define_prop_uint32!(c"node-id".as_ptr(), VirtmcuTelemetryQOM, node_id, 0),
        define_prop_string!(c"transport".as_ptr(), VirtmcuTelemetryQOM, transport),
        define_prop_string!(c"router".as_ptr(), VirtmcuTelemetryQOM, router),
    ]
);

static TELEMETRY_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"telemetry".as_ptr(),
    parent: TYPE_SYS_BUS_DEVICE,
    instance_size: core::mem::size_of::<VirtmcuTelemetryQOM>(),
    instance_align: 0,
    instance_init: Some(telemetry_init),
    instance_post_init: None,
    instance_finalize: None,
    abstract_: false,
    class_size: core::mem::size_of::<virtmcu_qom::qdev::SysBusDeviceClass>(),
    class_init: Some(telemetry_class_init),
    class_base_init: None,
    class_data: ptr::null(),
    interfaces: ptr::null(),
};

declare_device_type!(virtmcu_telemetry_register_types, TELEMETRY_TYPE_INFO);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_telemetry_qom_layout() {
        assert_eq!(
            core::mem::offset_of!(VirtmcuTelemetryQOM, parent_obj),
            0,
            "SysBusDevice must be the first field"
        );
    }
}
