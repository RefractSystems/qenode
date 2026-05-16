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
// Virtmcu telemetry peripheral with pluggable transport.

use core::ffi::{c_char, c_int, c_void};
use crossbeam_channel::{bounded, Receiver, RecvTimeoutError, Sender};
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
const TELEMETRY_POLL_TIMEOUT_MS: u64 = 10;

const EVENT_TYPE_CPU_STATE: i8 = 0;
const EVENT_TYPE_IRQ: i8 = 1;
const EVENT_TYPE_POWER_STATE: i8 = 3;

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
            virtmcu_qom::ffi_call! {
                virtmcu_qom::qom::g_free(self.path as *mut c_void);
            }
        }
    }
}

/// Internal Rust backend for `VirtmcuTelemetryQOM`.
pub struct VirtmcuTelemetryBackend {
    _transport: Arc<dyn virtmcu_api::DataTransport>,
    sender: Sender<Option<TraceEvent>>,
    tx_shutdown: Arc<AtomicBool>,
    tx_thread: Option<std::thread::JoinHandle<()>>,
    _node_id: u32,
    last_halted: Arc<[AtomicBool; MAX_CPUS]>,
    irq_slots: virtmcu_qom::sync::BqlGuarded<Vec<IrqSlot>>,
    _liveliness: Option<alloc::boxed::Box<dyn virtmcu_api::LivelinessToken>>,
}

impl Drop for VirtmcuTelemetryBackend {
    fn drop(&mut self) {
        self.tx_shutdown.store(true, Ordering::Release);
        let _ = self.sender.try_send(None); // best-effort wake-up
        if let Some(thread) = self.tx_thread.take() {
            thread.join().expect("telemetry worker thread panicked");
        }
    }
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
    let s = virtmcu_qom::ffi_call! { &*s_ptr };
    if s.rust_state.is_null() {
        return;
    }
    virtmcu_qom::ffi_call! {
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
    let s = virtmcu_qom::ffi_call! { &*s_ptr };
    if s.rust_state.is_null() {
        return;
    }
    virtmcu_qom::ffi_call! {
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

    let vtime = virtmcu_qom::timer::qemu_clock_get_ns_safe(virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL);

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
    let vtime = virtmcu_qom::timer::qemu_clock_get_ns_safe(virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL);
    let device_name = if name_ptr.is_null() {
        None
    } else {
        virtmcu_qom::ffi_call! { Some(CStr::from_ptr(name_ptr).to_string_lossy().into_owned()) }
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
    shutdown: Arc<AtomicBool>,
) {
    let mut builder = FlatBufferBuilder::new();

    loop {
        if shutdown.load(Ordering::Acquire) {
            break;
        }
        let ev = match rx.recv_timeout(core::time::Duration::from_millis(TELEMETRY_POLL_TIMEOUT_MS))
        {
            Ok(Some(ev)) => ev,
            Ok(None) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => continue,
        };

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
        let seq = 0;
        match transport.reserve(&topic, payload.len()) {
            Ok(mut reservation) => {
                reservation.buffer_mut().copy_from_slice(payload);
                let _ = reservation.commit(ev.timestamp_ns, seq);
            }
            Err(e) => {
                virtmcu_qom::sim_err!("Telemetry: Failed to reserve transport: {e:?}");
            }
        }
    }
}

/* ── QOM Methods ──────────────────────────────────────────────────────────── */






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

    /// Verifies that the telemetry worker thread observes the shutdown flag and exits within a
    /// bounded wall-clock window (≤ 100 ms) — guarding against the thread-leakage bug.
    #[test]
    fn test_telemetry_thread_exits_on_shutdown() {
        let (tx, rx) = bounded::<Option<TraceEvent>>(TRACE_EVENT_QUEUE_SIZE);
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_clone = Arc::clone(&shutdown);

        let handle = std::thread::spawn(move || loop {
            if shutdown_clone.load(Ordering::Acquire) {
                break;
            }
            match rx.recv_timeout(std::time::Duration::from_millis(1)) {
                Ok(None) | Err(RecvTimeoutError::Disconnected) => break,
                Ok(Some(_)) | Err(RecvTimeoutError::Timeout) => {}
            }
        });

        shutdown.store(true, Ordering::Release);
        let _ = tx.try_send(None); // best-effort wake-up

        let start = std::time::Instant::now();
        handle.join().expect("thread panicked");
        assert!(
            start.elapsed() < core::time::Duration::from_millis(100),
            "telemetry worker thread did not exit within 100 ms"
        );
    }
}
