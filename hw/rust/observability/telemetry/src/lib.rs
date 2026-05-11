#![cfg_attr(
    test,
    allow(
        clippy::expect_used,
        clippy::unwrap_used,
        clippy::panic,
        clippy::indexing_slicing,
        clippy::panic_in_result_fn
    )
)]
unsafe extern "C" fn allow_set_link(
    _obj: *mut virtmcu_qom::qom::Object,
    _name: *const core::ffi::c_char,
    _val: *mut virtmcu_qom::qom::Object,
    _errp: *mut *mut virtmcu_qom::error::Error,
) {
}
// Virtmcu telemetry peripheral with pluggable transport.

use core::ffi::{c_char, c_int, c_void};
use crossbeam_channel::{bounded, Receiver, Sender};
use flatbuffers::FlatBufferBuilder;
extern crate alloc;
use alloc::sync::Arc;
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use virtmcu_api::TraceEvent;
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

/* ── QOM Object ───────────────────────────────────────────────────────────── */

const MAX_CPUS: usize = 32;
const TRACE_EVENT_QUEUE_SIZE: usize = 10000;
const IRQ_ID_SLOT_SHIFT: u32 = 16;

const EVENT_TYPE_CPU_STATE: u8 = 0;
const EVENT_TYPE_IRQ: u8 = 1;
const EVENT_TYPE_POWER_STATE: u8 = 3;

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

    /* Links */
    pub transport_hub: *mut Object,

    /* Rust state */
    /// Opaque pointer to the Rust backend state.
    pub rust_state: *mut VirtmcuTelemetryBackend,
}

const _: () = assert!(core::mem::offset_of!(VirtmcuTelemetryQOM, parent_obj) == 0);

struct IrqSlot {
    #[allow(dead_code)] // virtmcu-allow: allow reasoning="Reserved for future GPIO support"
    opaque: *mut c_void,
    #[allow(dead_code)] // virtmcu-allow: allow reasoning="Reserved for future GPIO support"
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
    last_halted: Arc<[AtomicBool; MAX_CPUS]>,
    irq_slots: virtmcu_qom::sync::BqlGuarded<Vec<IrqSlot>>,
    _liveliness: Option<alloc::boxed::Box<dyn virtmcu_api::LivelinessToken>>,
}

// SAFETY: VirtmcuTelemetryBackend encapsulates cross-thread channel sender and atomic state.
unsafe impl Send for VirtmcuTelemetryBackend {}
// SAFETY: VirtmcuTelemetryBackend's fields are internally synchronized (Atomic, Sender, BqlGuarded).
unsafe impl Sync for VirtmcuTelemetryBackend {}

static GLOBAL_TELEMETRY: AtomicPtr<VirtmcuTelemetryQOM> = AtomicPtr::new(ptr::null_mut()); // virtmcu-allow: static_state reasoning="Required for C-FFI hook dispatch"

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
        let cpu_index = virtmcu_qom::cpu::virtmcu_cpu_get_index(cpu);
        telemetry_trace_cpu_internal(backend, cpu_index, halted);
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
    if !(0..MAX_CPUS as c_int).contains(&cpu_index) {
        return;
    }

    // Only trace if state actually changed
    let cpu_idx = usize::try_from(cpu_index).expect("invalid cpu index");
    let prev = backend
        .last_halted
        .get(cpu_idx)
        .expect("CPU index out of range")
        .swap(halted, Ordering::SeqCst);
    if prev == halted {
        return;
    }

    let vtime = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };

    let _ = backend.sender.try_send(Some(TraceEvent {
        timestamp_ns: vtime.try_into().expect("vtime is negative"),
        event_type: EVENT_TYPE_CPU_STATE,
        id: cpu_index.try_into().expect("cpu_index is negative"),
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
    let id = (u32::from(slot) << IRQ_ID_SLOT_SHIFT) | u32::from(pin);
    let vtime = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };
    let device_name = if name_ptr.is_null() {
        None
    } else {
        unsafe { Some(CStr::from_ptr(name_ptr).to_string_lossy().into_owned()) }
    };

    let _ = backend.sender.try_send(Some(TraceEvent {
        timestamp_ns: vtime as u64,
        event_type: EVENT_TYPE_IRQ,
        id,
        value: level as u32,
        device_name,
        power_uw: 0,
    }));
}
*/

use virtmcu_api::telemetry_generated::virtmcu::telemetry::{
    TraceEvent as GenTraceEvent, TraceEventArgs, TraceEventType,
};

fn telemetry_worker(
    rx: Receiver<Option<TraceEvent>>,
    transport: Arc<dyn virtmcu_api::DataTransport>,
    topic: String,
) {
    let mut builder = FlatBufferBuilder::new();

    while let Ok(Some(ev)) = rx.recv() {
        builder.reset();

        let device_name_off = ev.device_name.as_deref().map(|s| builder.create_string(s));

        let args = TraceEventArgs {
            timestamp_ns: ev.timestamp_ns,
            type_: match ev.event_type {
                EVENT_TYPE_CPU_STATE => TraceEventType::CPU_STATE,
                EVENT_TYPE_IRQ => TraceEventType::IRQ,
                EVENT_TYPE_POWER_STATE => TraceEventType::POWER_STATE,
                _ => TraceEventType::PERIPHERAL,
            },
            id: ev.id,
            value: ev.value,
            device_name: device_name_off,
            power_uw: ev.power_uw,
        };

        let root = GenTraceEvent::create(&mut builder, &args);
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
    s.transport_hub = ptr::null_mut();
}

unsafe extern "C" fn telemetry_realize(dev_state: *mut c_void, _errp: *mut *mut c_void) {
    let dev = dev_state as *mut virtmcu_qom::qdev::DeviceState;
    let s = &mut *(dev as *mut VirtmcuTelemetryQOM);

    if !s.rust_state.is_null() {
        return;
    }

    if s.transport_hub.is_null() {
        error_setg!(
            _errp as *mut *mut virtmcu_qom::error::Error,
            "Strict DI violation: transport_hub link is required."
        );
        return;
    }

    unsafe {
        virtmcu_qom::qom::object_property_set_bool(
            s.transport_hub,
            c"realized".as_ptr(),
            true,
            core::ptr::null_mut(),
        );
    }
    let ptr_u64 = unsafe {
        virtmcu_qom::qom::object_property_get_uint(
            s.transport_hub,
            c"transport_ptr".as_ptr(),
            core::ptr::null_mut(),
        )
    };
    if ptr_u64 == 0 {
        error_setg!(
            _errp as *mut *mut virtmcu_qom::error::Error,
            "Strict DI violation: failed to acquire transport from hub."
        );
        return;
    }
    let transport_ref =
        unsafe { &*(ptr_u64 as *const alloc::sync::Arc<dyn virtmcu_api::DataTransport>) };
    let transport = alloc::sync::Arc::clone(transport_ref);

    let (tx, rx) = bounded(TRACE_EVENT_QUEUE_SIZE);
    let node_id = s.node_id;
    let topic = format!("sim/telemetry/trace/{node_id}");

    let key = format!("sim/telemetry/liveliness/{node_id}");
    let _liveliness = transport.declare_liveliness(&key);

    let backend = Box::new(VirtmcuTelemetryBackend {
        _transport: Arc::clone(&transport),
        sender: tx,
        _node_id: node_id,
        last_halted: Arc::new(Default::default()),
        irq_slots: virtmcu_qom::sync::BqlGuarded::new(Vec::new()),
        _liveliness,
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

    let slot = u16::try_from(backend.len()).expect("too many IRQ slots");

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
        (*dc).user_creatable = true;
    }

    device_class_set_props!(dc, TELEMETRY_PROPERTIES);

    unsafe {
        virtmcu_qom::qom::object_class_property_add_link(
            klass,
            c"transport".as_ptr(),
            c"virtmcu-transport-hub".as_ptr(),
            core::mem::offset_of!(VirtmcuTelemetryQOM, transport_hub) as isize,
            Some(allow_set_link),
            virtmcu_qom::qom::OBJ_PROP_LINK_STRONG,
        );
    }
}

define_properties!(
    TELEMETRY_PROPERTIES,
    [
        define_prop_uint32!(c"node".as_ptr(), VirtmcuTelemetryQOM, node_id, 0),
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
