#![allow(clippy::panic)] // virtmcu-allow: allow reasoning="Fail Loudly"
#![cfg_attr(
    test,
    allow(
        clippy::expect_used,
        clippy::unwrap_used,
        clippy::indexing_slicing,
        clippy::panic_in_result_fn
    )
)]
#![allow(clippy::print_stderr)] // virtmcu-allow: print_stderr reasoning="Startup error reporting"
//! Enterprise Continuous High-Fidelity TCG Instruction Tracer

mod qemu_plugin;
use qemu_plugin::{
    qemu_plugin_insn_disas, qemu_plugin_insn_vaddr, qemu_plugin_outs,
    qemu_plugin_register_atexit_cb, qemu_plugin_register_vcpu_insn_exec_cb,
    qemu_plugin_register_vcpu_tb_trans_cb, qemu_plugin_tb_get_insn, qemu_plugin_tb_n_insns,
    QemuInfo, QemuPluginCbFlags, QemuPluginId, QemuPluginTb,
};

/// Log through QEMU's own output channel.  Safe before any tracing subscriber exists.
macro_rules! plugin_log {
    ($($arg:tt)*) => {{
        let msg = alloc::format!("{}\0", alloc::format!($($arg)*));
        unsafe { qemu_plugin_outs(msg.as_ptr().cast()) };
    }};
}

extern crate alloc;

use alloc::borrow::ToOwned;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ffi::{c_char, c_int, c_uint, c_void, CStr};
use core::sync::atomic::{AtomicBool, Ordering};
use crossbeam_channel::{unbounded, Sender};
use flatbuffers::FlatBufferBuilder;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::thread::{spawn, JoinHandle};
use virtmcu_api::insn_trace_generated::virtmcu::insn_trace::{InsnTrace, InsnTraceArgs};
use virtmcu_api::topics::sim_topic;
use virtmcu_api::DataTransport;

// virtmcu-allow: static_state reasoning="QEMU TCG Plugin API lacks userdata for tb_trans."
static STATE: OnceLock<Arc<TracerState>> = OnceLock::new();

// virtmcu-allow: static_state reasoning="High-performance lock-free execution toggle."
static GLOBAL_TRACE_ENABLED: AtomicBool = AtomicBool::new(false);

/// Required by QEMU to load the plugin.
#[no_mangle]
pub static qemu_plugin_version: c_int = 2;

#[derive(Clone, Copy)]
struct ExecEvent {
    vtime: u64,
    pc: u64,
}

// Raw pointer to InsnData allocated on the heap and passed to QEMU as callback userdata.
// Safety: access is serialized through Mutex; QEMU guarantees the ptr lives until plugin_exit.
struct InsnPtr(*mut InsnData);
unsafe impl Send for InsnPtr {}
unsafe impl Sync for InsnPtr {}

struct TracerState {
    // virtmcu-allow: mutex reasoning="Drop management for clean shutdown"
    tx_master: Mutex<Option<Sender<ExecEvent>>>,
    // virtmcu-allow: mutex reasoning="Plugin cache shared across tb_trans threads"
    disas_cache: Mutex<HashMap<u64, String>>,
    // virtmcu-allow: mutex reasoning="Raw QEMU callback pointers, freed in plugin_exit"
    insn_contexts: Mutex<Vec<InsnPtr>>,
    // virtmcu-allow: mutex reasoning="Thread management for clean join on exit"
    worker_handle: Mutex<Option<JoinHandle<()>>>,
    // virtmcu-allow: mutex reasoning="OTel guard to ensure logs are flushed on plugin exit"
    _telemetry_guard: Mutex<Option<virtmcu_observability::OTelGuard>>,
}

struct InsnData {
    pc: u64,
    tx: Sender<ExecEvent>,
}

/// RAII Wrapper for strings allocated by QEMU that must be freed via libc.
struct QemuString(*mut c_char);
impl Drop for QemuString {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { libc::free(self.0.cast::<c_void>()) };
        }
    }
}
impl QemuString {
    fn to_string_lossy(&self) -> String {
        if self.0.is_null() {
            "Unknown".to_owned()
        } else {
            unsafe { CStr::from_ptr(self.0) }.to_string_lossy().into_owned()
        }
    }
}

#[derive(Debug)]
struct DummyVTimeProvider;

impl virtmcu_observability::processors::VTimeProvider for DummyVTimeProvider {
    fn current_vtime_ns(&self) -> u64 {
        // TCG Tracer runs locally without a concept of the global vtime currently
        // Alternatively, this could fetch the VCPU's icount if that was exposed.
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn qemu_plugin_install(
    id: QemuPluginId,
    _info: *const QemuInfo,
    nargs: c_int,
    params: *mut *mut c_char,
) -> c_int {
    let mut node_id = 0;
    let mut transport_cfg = "zenoh".to_owned();

    for i in 0..nargs {
        let cstr_ptr = *params.add(i as usize);
        if !cstr_ptr.is_null() {
            let decoded = CStr::from_ptr(cstr_ptr).to_string_lossy();
            if let Some(val) = decoded.strip_prefix("node_id=") {
                node_id = val.parse().expect("Invalid data format");
            } else if let Some(val) = decoded.strip_prefix("transport=") {
                val.clone_into(&mut transport_cfg);
            }
        }
    }

    let transport: Arc<dyn DataTransport> = if let Some(path) = transport_cfg.strip_prefix("unix:")
    {
        match transport_unix::UdsDataTransport::new(path) {
            Ok(t) => Arc::new(t),
            Err(e) => {
                plugin_log!("tcg-tracer: Failed to initialize Unix transport: {e:?}");
                return -1;
            }
        }
    } else {
        match transport_zenoh::get_or_init_session(core::ptr::null()) {
            Ok(s) => Arc::new(transport_zenoh::ZenohDataTransport::new(s)),
            Err(e) => {
                plugin_log!("tcg-tracer: Failed to initialize Zenoh transport: {e:?}");
                return -1;
            }
        }
    };

    let (tx, rx) = unbounded::<ExecEvent>();
    let handle = spawn(move || {
        background_stream_worker(rx, transport, node_id);
    });

    let service_name = alloc::format!("virtmcu-qemu-plugin-{node_id}");
    let service_name_static: &'static str = Box::leak(service_name.into_boxed_str());
    let guard = virtmcu_observability::init_plugin_telemetry(
        service_name_static,
        Arc::new(DummyVTimeProvider),
    );

    let state = Arc::new(TracerState {
        tx_master: Mutex::new(Some(tx)),
        disas_cache: Mutex::new(HashMap::new()),
        insn_contexts: Mutex::new(Vec::new()),
        worker_handle: Mutex::new(Some(handle)),
        _telemetry_guard: Mutex::new(Some(guard)),
    });

    if STATE.set(state).is_err() {
        plugin_log!("tcg-tracer: Failed to set global STATE (already set?)");
        return -1;
    }

    tracing::info!("tcg-tracer: OTel telemetry initialized for node {}", node_id);

    qemu_plugin_register_vcpu_tb_trans_cb(id, vcpu_tb_trans);
    qemu_plugin_register_atexit_cb(id, plugin_exit, core::ptr::null_mut());

    GLOBAL_TRACE_ENABLED.store(true, Ordering::Relaxed);
    0
}

unsafe extern "C" fn vcpu_tb_trans(_id: QemuPluginId, tb: *mut QemuPluginTb) {
    let state = match STATE.get() {
        Some(s) => s,
        None => return,
    };

    let tx = if let Ok(guard) = state.tx_master.lock() {
        if let Some(t) = &*guard {
            t.clone()
        } else {
            return;
        }
    } else {
        return;
    };

    let n_insns = qemu_plugin_tb_n_insns(tb);
    for i in 0..n_insns {
        let insn = qemu_plugin_tb_get_insn(tb, i);
        let pc = qemu_plugin_insn_vaddr(insn);
        let raw_disas = qemu_plugin_insn_disas(insn);

        let qstr = QemuString(raw_disas);
        let disas = qstr.to_string_lossy();

        if let Ok(mut cache) = state.disas_cache.lock() {
            cache.insert(pc, disas);
        }

        let insn_ptr = Box::into_raw(Box::new(InsnData { pc, tx: tx.clone() }));

        if let Ok(mut contexts) = state.insn_contexts.lock() {
            contexts.push(InsnPtr(insn_ptr));
        }

        qemu_plugin_register_vcpu_insn_exec_cb(
            insn,
            vcpu_insn_exec,
            QemuPluginCbFlags::NoRegs,
            insn_ptr.cast::<c_void>(),
        );
    }
}

unsafe extern "C" fn vcpu_insn_exec(_vcpu_index: c_uint, userdata: *mut c_void) {
    if !GLOBAL_TRACE_ENABLED.load(Ordering::Relaxed) {
        return;
    }
    if userdata.is_null() {
        return;
    }
    let insn_data = &*(userdata.cast::<InsnData>());
    let event = ExecEvent { vtime: 0, pc: insn_data.pc };
    let _ = insn_data.tx.try_send(event);
}

unsafe extern "C" fn plugin_exit(_id: QemuPluginId, _userdata: *mut c_void) {
    if let Some(state) = STATE.get() {
        // Drop the master sender
        if let Ok(mut tx_opt) = state.tx_master.lock() {
            tx_opt.take();
        }
        // Drop all QEMU callback heap allocations from the translation phase
        if let Ok(mut contexts) = state.insn_contexts.lock() {
            for InsnPtr(ptr) in contexts.drain(..) {
                drop(unsafe { Box::from_raw(ptr) });
            }
        }
        // Wait for the background worker to flush the queue and cleanly exit
        if let Ok(mut handle_opt) = state.worker_handle.lock() {
            if let Some(handle) = handle_opt.take() {
                let _ = handle.join();
            }
        }
        // Flush and shutdown telemetry pipelines
        if let Ok(mut guard_opt) = state._telemetry_guard.lock() {
            let _ = guard_opt.take(); // Drops OTelGuard
        }
    }
}

fn background_stream_worker(
    rx: crossbeam_channel::Receiver<ExecEvent>,
    transport: Arc<dyn DataTransport>,
    node_id: u32,
) {
    let node_id_str = node_id.to_string();
    let topic = sim_topic::telemetry_insn(&node_id_str);
    let mut builder = FlatBufferBuilder::new();

    while let Ok(event) = rx.recv() {
        let disas = if let Some(state) = STATE.get() {
            if let Ok(cache) = state.disas_cache.lock() {
                cache.get(&event.pc).cloned().unwrap_or_else(|| "Unknown".to_owned())
            } else {
                "Unknown".to_owned()
            }
        } else {
            "Unknown".to_owned()
        };

        builder.reset();
        let disas_fb = builder.create_string(&disas);
        let args = InsnTraceArgs {
            timestamp_ns: event.vtime,
            pc: event.pc,
            disassembly: Some(disas_fb),
            quantum_number: 0u64, // Not yet provided by TCG plugin
        };
        let insn_trace = InsnTrace::create(&mut builder, &args);
        virtmcu_api::insn_trace_generated::virtmcu::insn_trace::finish_insn_trace_buffer(
            &mut builder,
            insn_trace,
        );
        let _ = transport.publish(&topic, builder.finished_data());
    }
}
