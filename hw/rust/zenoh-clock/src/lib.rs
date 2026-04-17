#![allow(clippy::missing_safety_doc, clippy::collapsible_match, dead_code, unused_imports, clippy::len_zero, clippy::while_immutable_condition)]
extern crate libc;

use core::ffi::{c_char, c_void};
use std::ffi::CStr;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use zenoh::{Config, Session, Wait};
use zenoh::query::{Query, Queryable};

#[repr(C, packed)]
#[derive(Debug, Copy, Clone, Default)]
pub struct ClockAdvanceReq {
    pub delta_ns: u64,
    pub mujoco_time_ns: u64,
}

#[repr(C, packed)]
#[derive(Debug, Copy, Clone, Default)]
pub struct ClockReadyResp {
    pub current_vtime_ns: u64,
    pub n_frames: u32,
    pub error_code: u32, // 0=OK, 1=STALL
}

pub struct ZenohClockBackend {
    session: Session,
    queryable: Option<Queryable<()>>,
    node_id: u32,
    stall_timeout_ms: u32,
    
    // Shared state with QEMU vCPU thread
    mutex: *mut c_void,
    vcpu_cond: *mut c_void,
    query_cond: *mut c_void,
    
    delta_ns: Arc<AtomicI64>,
    mujoco_time_ns: Arc<AtomicI64>,
    vtime_ns: Arc<AtomicI64>,
    quantum_ready: Arc<AtomicBool>,
    quantum_done: Arc<AtomicBool>,
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_clock_init(
    node_id: u32,
    router: *const c_char,
    stall_timeout_ms: u32,
    mutex: *mut c_void,
    vcpu_cond: *mut c_void,
    query_cond: *mut c_void,
) -> *mut ZenohClockBackend {
    let mut config = Config::default();
    if !router.is_null() {
        if let Ok(r_str) = CStr::from_ptr(router).to_str() {
            if !r_str.is_empty() {
                let json = format!("[\"{}\"]", r_str);
                let _ = config.insert_json5("connect/endpoints", &json);
                let _ = config.insert_json5("scouting/multicast/enabled", "false");
            }
        }
    }

    let session = match zenoh::open(config).wait() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("failed to open Zenoh session: {:?}", e);
            return ptr::null_mut();
        }
    };

    let delta_ns = Arc::new(AtomicI64::new(0));
    let mujoco_time_ns = Arc::new(AtomicI64::new(0));
    let vtime_ns = Arc::new(AtomicI64::new(0));
    let quantum_ready = Arc::new(AtomicBool::new(false));
    let quantum_done = Arc::new(AtomicBool::new(false));

    let backend_ptr = Box::into_raw(Box::new(ZenohClockBackend {
        session: session.clone(),
        queryable: None,
        node_id,
        stall_timeout_ms,
        mutex,
        vcpu_cond,
        query_cond,
        delta_ns: delta_ns.clone(),
        mujoco_time_ns: mujoco_time_ns.clone(),
        vtime_ns: vtime_ns.clone(),
        quantum_ready: quantum_ready.clone(),
        quantum_done: quantum_done.clone(),
    }));

    let topic = format!("sim/clock/advance/{}", node_id);
    let backend_usize = backend_ptr as usize;
    let queryable = session.declare_queryable(topic)
        .callback(move |query| {
            let backend = unsafe { &*(backend_usize as *const ZenohClockBackend) };
            on_clock_query(backend, query);
        })
        .wait()
        .unwrap();

    // Assign queryable back to backend
    let b = &mut *backend_ptr;
    b.queryable = Some(queryable);

    backend_ptr
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_clock_free(backend: *mut ZenohClockBackend) {
    if !backend.is_null() {
        let _ = Box::from_raw(backend);
    }
}

unsafe fn on_clock_query(backend: &ZenohClockBackend, query: Query) {
    let payload = match query.payload() {
        Some(p) => p,
        None => { reply_error(query, 2); return; }
    };

    if payload.len() < 16 {
        reply_error(query, 2);
        return;
    }

    let mut delta: u64 = 0;
    let mut mujoco: u64 = 0;
    let data = payload.to_bytes();
    ptr::copy_nonoverlapping(data.as_ptr(), &mut delta as *mut u64 as *mut u8, 8);
    ptr::copy_nonoverlapping(data.as_ptr().add(8), &mut mujoco as *mut u64 as *mut u8, 8);

    backend.delta_ns.store(delta as i64, Ordering::Release);
    backend.mujoco_time_ns.store(mujoco as i64, Ordering::Release);

    virtmcu_mutex_lock(backend.mutex);
    backend.quantum_done.store(false, Ordering::Release);
    backend.quantum_ready.store(true, Ordering::Release);
    virtmcu_cond_signal(backend.vcpu_cond);

    let mut error_code = 0;
    while !backend.quantum_done.load(Ordering::Acquire) {
        let rc = virtmcu_cond_timedwait(backend.query_cond, backend.mutex, backend.stall_timeout_ms);
        if rc == 0 && !backend.quantum_done.load(Ordering::Acquire) {
            error_code = 1; // STALL
            break;
        }
    }

    let vtime = backend.vtime_ns.load(Ordering::Acquire);
    virtmcu_mutex_unlock(backend.mutex);

    let resp = ClockReadyResp {
        current_vtime_ns: vtime as u64,
        n_frames: 0,
        error_code,
    };
    
    let resp_bytes = std::slice::from_raw_parts(&resp as *const _ as *const u8, 16);
    let _ = query.reply(query.key_expr(), resp_bytes).wait();
}

fn reply_error(query: Query, error_code: u32) {
    let resp = ClockReadyResp {
        current_vtime_ns: 0,
        n_frames: 0,
        error_code,
    };
    let resp_bytes = unsafe {
        std::slice::from_raw_parts(&resp as *const _ as *const u8, 16)
    };
    let _ = query.reply(query.key_expr(), resp_bytes).wait();
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_clock_quantum_wait(backend: *mut ZenohClockBackend, current_vtime_ns: i64) -> i64 {
    let b = &*backend;
    
    virtmcu_mutex_lock(b.mutex);
    b.vtime_ns.store(current_vtime_ns, Ordering::Release);
    b.quantum_done.store(true, Ordering::Release);
    virtmcu_cond_signal(b.query_cond);

    while !b.quantum_ready.load(Ordering::Acquire) {
        virtmcu_cond_wait(b.vcpu_cond, b.mutex);
    }

    b.quantum_ready.store(false, Ordering::Release);
    let delta = b.delta_ns.load(Ordering::Acquire);
    virtmcu_mutex_unlock(b.mutex);

    delta
}

extern "C" {
    fn virtmcu_mutex_lock(mutex: *mut c_void);
    fn virtmcu_mutex_unlock(mutex: *mut c_void);
    fn virtmcu_cond_signal(cond: *mut c_void);
    fn virtmcu_cond_wait(cond: *mut c_void, mutex: *mut c_void);
    fn virtmcu_cond_timedwait(cond: *mut c_void, mutex: *mut c_void, ms: u32) -> i32;
}
