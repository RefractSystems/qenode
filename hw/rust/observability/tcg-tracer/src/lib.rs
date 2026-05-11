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
//! Enterprise Continuous High-Fidelity TCG Instruction Tracer

mod qemu_plugin;
use qemu_plugin::{
    qemu_plugin_insn_disas, qemu_plugin_insn_vaddr, qemu_plugin_register_atexit_cb,
    qemu_plugin_register_vcpu_insn_exec_cb, qemu_plugin_register_vcpu_tb_trans_cb,
    qemu_plugin_tb_get_insn, qemu_plugin_tb_n_insns, QemuInfo, QemuPluginCbFlags, QemuPluginId,
    QemuPluginTb,
};

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
use virtmcu_api::telemetry_generated::virtmcu::telemetry::{
    TraceEvent, TraceEventArgs, TraceEventType,
};
use virtmcu_api::DataTransport;

// virtmcu-allow: static_state reasoning="QEMU TCG Plugin API lacks userdata for tb_trans."
static STATE: OnceLock<Arc<TracerState>> = OnceLock::new();

// virtmcu-allow: static_state reasoning="High-performance lock-free execution toggle."
static GLOBAL_TRACE_ENABLED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy)]
struct ExecEvent {
    vtime: u64,
    pc: u64,
}

struct TracerState {
    // virtmcu-allow: mutex reasoning="Drop management for clean shutdown"
    tx_master: Mutex<Option<Sender<ExecEvent>>>,
    // virtmcu-allow: mutex reasoning="Plugin cache shared across tb_trans threads"
    disas_cache: Mutex<HashMap<u64, String>>,
    // virtmcu-allow: mutex reasoning="Heap contexts tracking for memory leak prevention"
    insn_contexts: Mutex<Vec<Box<InsnData>>>,
    // virtmcu-allow: mutex reasoning="Thread management for clean join on exit"
    worker_handle: Mutex<Option<JoinHandle<()>>>,
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

#[no_mangle]
pub static qemu_plugin_version: c_int = 2;

#[no_mangle]
pub unsafe extern "C" fn qemu_plugin_install(
    id: QemuPluginId,
    _info: *const QemuInfo,
    argc: c_int,
    argv: *mut *mut c_char,
) -> c_int {
    let mut node_id = 0;
    let mut transport_cfg = "zenoh".to_owned();

    for i in 0..argc {
        let arg_ptr = *argv.add(i as usize);
        if !arg_ptr.is_null() {
            let arg_str = CStr::from_ptr(arg_ptr).to_string_lossy();
            if let Some(val) = arg_str.strip_prefix("node_id=") {
                node_id = val.parse().unwrap_or(0);
            } else if let Some(val) = arg_str.strip_prefix("transport=") {
                transport_cfg = val.to_owned();
            }
        }
    }

    let transport: Arc<dyn DataTransport> = if let Some(path) = transport_cfg.strip_prefix("unix:")
    {
        match transport_unix::UnixDataTransport::new(path) {
            Ok(t) => Arc::new(t),
            Err(e) => {
                println!("tcg-tracer: Failed to initialize Unix transport: {:?}", e);
                return -1;
            }
        }
    } else {
        match transport_zenoh::get_or_init_session(core::ptr::null()) {
            Ok(s) => Arc::new(transport_zenoh::ZenohDataTransport::new(s)),
            Err(e) => {
                println!("tcg-tracer: Failed to initialize Zenoh transport: {:?}", e);
                return -1;
            }
        }
    };

    let (tx, rx) = unbounded::<ExecEvent>();
    let handle = spawn(move || {
        background_stream_worker(rx, transport, node_id);
    });

    let state = Arc::new(TracerState {
        tx_master: Mutex::new(Some(tx)),
        disas_cache: Mutex::new(HashMap::new()),
        insn_contexts: Mutex::new(Vec::new()),
        worker_handle: Mutex::new(Some(handle)),
    });

    if STATE.set(state).is_err() {
        println!("tcg-tracer: Failed to set global STATE (already set?)");
        return -1;
    }

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
        let disas_ptr = qemu_plugin_insn_disas(insn);

        let qstr = QemuString(disas_ptr);
        let disas_str = qstr.to_string_lossy();

        if let Ok(mut cache) = state.disas_cache.lock() {
            cache.insert(pc, disas_str);
        }

        let insn_data = Box::new(InsnData { pc, tx: tx.clone() });
        let insn_ptr = Box::into_raw(insn_data);

        if let Ok(mut contexts) = state.insn_contexts.lock() {
            contexts.push(Box::from_raw(insn_ptr));
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
        // Drop all cloned senders from the translation phase
        if let Ok(mut contexts) = state.insn_contexts.lock() {
            contexts.clear();
        }
        // Wait for the background worker to flush the queue and cleanly exit
        if let Ok(mut handle_opt) = state.worker_handle.lock() {
            if let Some(handle) = handle_opt.take() {
                let _ = handle.join();
            }
        }
    }
}

fn background_stream_worker(
    rx: crossbeam_channel::Receiver<ExecEvent>,
    transport: Arc<dyn DataTransport>,
    node_id: u32,
) {
    let topic = alloc::format!("sim/telemetry/trace/{node_id}/insn");
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
        let args = TraceEventArgs {
            timestamp_ns: event.vtime,
            type_: TraceEventType::CPU_STATE,
            id: event.pc as u32,
            value: 0,
            device_name: Some(disas_fb),
            power_uw: 0,
        };
        let trace_event = TraceEvent::create(&mut builder, &args);
        builder.finish(trace_event, None);
        let _ = transport.publish(&topic, builder.finished_data());
    }
}
