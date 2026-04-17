#![allow(clippy::missing_safety_doc, clippy::collapsible_match, dead_code, unused_imports, clippy::len_zero, clippy::while_immutable_condition)]
extern crate libc;

use core::ffi::{c_char, c_void};
use std::ffi::CStr;
use std::ptr;
use std::sync::atomic::{AtomicI64, AtomicBool, Ordering};

use zenoh::bytes::ZBytes;
use zenoh::query::Query;
use zenoh::query::Queryable;
use zenoh::Config;
use zenoh::Session;
use zenoh::Wait;

use virtmcu_qom::cpu::*;
use virtmcu_qom::proto::*;
use virtmcu_qom::sync::*;
use virtmcu_qom::timer::*;

pub struct ZenohClockState {
    #[allow(dead_code)]
    node_id: u32,
    #[allow(dead_code)]
    is_icount: bool,

    #[allow(dead_code)]
    session: Session,
    #[allow(dead_code)]
    queryable: Option<Queryable<()>>,

    quantum_timer: *mut QemuTimer,

    mutex: *mut QemuMutex,
    vcpu_cond: *mut QemuCond,
    query_cond: *mut QemuCond,

    delta_ns: AtomicI64,
    mujoco_time_ns: AtomicI64,
    quantum_start_vtime_ns: AtomicI64,
    needs_quantum_atomic: AtomicBool,

    inner: *mut ZenohClockInner,
}

struct ZenohClockInner {
    quantum_ready: bool,
    quantum_done: bool,
    vtime_ns: i64,
}

static mut GLOBAL_ZENOH_CLOCK: *mut ZenohClockState = ptr::null_mut();

#[no_mangle]
pub unsafe extern "C" fn zenoh_clock_init(
    node_id: u32,
    router: *const c_char,
    mode: *const c_char,
) -> *mut ZenohClockState {
    let mut config = Config::default();
    if !router.is_null() {
        let router_str = unsafe { CStr::from_ptr(router) }.to_str().unwrap();
        let json = format!("[\"{}\"]", router_str);
        let _ = config.insert_json5("connect/endpoints", &json);
        let _ = config.insert_json5("scouting/multicast/enabled", "false");
    }

    let session = match zenoh::open(config).wait() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[zenoh-clock] node={}: FAILED session: {}", node_id, e);
            return ptr::null_mut();
        }
    };

    let is_icount = if !mode.is_null() {
        let mode_str = unsafe { CStr::from_ptr(mode) }.to_str().unwrap();
        mode_str == "icount"
    } else {
        false
    };

    let mutex = unsafe { virtmcu_mutex_new() };
    let vcpu_cond = unsafe { virtmcu_cond_new() };
    let query_cond = unsafe { virtmcu_cond_new() };

    let inner = Box::into_raw(Box::new(ZenohClockInner {
        quantum_ready: false,
        quantum_done: false,
        vtime_ns: 0,
    }));

    let state_box = Box::new(ZenohClockState {
        node_id,
        is_icount,
        session: session.clone(),
        queryable: None,
        quantum_timer: ptr::null_mut(),
        mutex,
        vcpu_cond,
        query_cond,
        delta_ns: AtomicI64::new(0),
        mujoco_time_ns: AtomicI64::new(0),
        quantum_start_vtime_ns: AtomicI64::new(0),
        needs_quantum_atomic: AtomicBool::new(true),
        inner,
    });

    let state_ptr = Box::into_raw(state_box);
    unsafe { GLOBAL_ZENOH_CLOCK = state_ptr };

    let topic = format!("sim/clock/advance/{}", node_id);
    let state_ptr_for_zenoh = state_ptr as usize;

    let queryable = session
        .declare_queryable(topic)
        .callback(move |query| {
            let state = unsafe { &*(state_ptr_for_zenoh as *const ZenohClockState) };
            on_query(state, query);
        })
        .wait()
        .unwrap();

    unsafe {
        (*state_ptr).queryable = Some(queryable);
        (*state_ptr).quantum_timer = virtmcu_timer_new_ns(
            QEMU_CLOCK_VIRTUAL,
            zclock_timer_cb,
            state_ptr as *mut c_void,
        );
        virtmcu_tcg_quantum_hook = Some(zclock_quantum_hook);
        virtmcu_get_quantum_timing = Some(zclock_get_quantum_timing);
        virtmcu_cpu_exit_all();
    }

    state_ptr
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_clock_fini(state: *mut ZenohClockState) {
    if state.is_null() { return; }
    unsafe {
        if GLOBAL_ZENOH_CLOCK == state {
            GLOBAL_ZENOH_CLOCK = ptr::null_mut();
        }
        virtmcu_tcg_quantum_hook = None;
        virtmcu_get_quantum_timing = None;
        let s = Box::from_raw(state);
        if !s.quantum_timer.is_null() {
            virtmcu_timer_free(s.quantum_timer);
        }
        virtmcu_mutex_free(s.mutex);
        virtmcu_cond_free(s.vcpu_cond);
        virtmcu_cond_free(s.query_cond);
        let _inner = Box::from_raw(s.inner);
    }
}

extern "C" fn zclock_timer_cb(opaque: *mut c_void) {
    let state = unsafe { &*(opaque as *mut ZenohClockState) };
    state.needs_quantum_atomic.store(true, Ordering::Release);
    unsafe {
        virtmcu_cpu_exit_all();
    }
}

extern "C" fn zclock_get_quantum_timing(timing: *mut VirtmcuQuantumTiming) {
    unsafe {
        if GLOBAL_ZENOH_CLOCK.is_null() || timing.is_null() {
            return;
        }
        let s = &*GLOBAL_ZENOH_CLOCK;
        (*timing).quantum_start_vtime_ns = s.quantum_start_vtime_ns.load(Ordering::Acquire);
        (*timing).quantum_delta_ns = s.delta_ns.load(Ordering::Acquire);
        (*timing).mujoco_time_ns = s.mujoco_time_ns.load(Ordering::Acquire);
    }
}

extern "C" fn zclock_quantum_hook(_cpu: *mut CPUState) {
    let state = unsafe {
        if GLOBAL_ZENOH_CLOCK.is_null() { return; }
        &*GLOBAL_ZENOH_CLOCK
    };

    if !state.needs_quantum_atomic.load(Ordering::Acquire) {
        return;
    }

    unsafe {
        virtmcu_mutex_lock(state.mutex);
        // Signal done
        virtmcu_bql_lock();
        state.needs_quantum_atomic.store(false, Ordering::Release);
        (*state.inner).vtime_ns = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL);
        (*state.inner).quantum_done = true;
        virtmcu_cond_signal(state.query_cond);
        virtmcu_bql_unlock();

        // Wait for ready
        while !(*state.inner).quantum_ready {
            virtmcu_cond_wait(state.vcpu_cond, state.mutex);
        }

        (*state.inner).quantum_ready = false;
        (*state.inner).quantum_done = false;
        
        let next_delta = state.delta_ns.load(Ordering::Acquire);

        virtmcu_bql_lock();
        let vtime_now = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL);
        (*state.inner).vtime_ns = vtime_now;
        state.quantum_start_vtime_ns.store(vtime_now, Ordering::Release);

        virtmcu_timer_mod(state.quantum_timer, vtime_now + next_delta);
        virtmcu_bql_unlock();

        
        virtmcu_mutex_unlock(state.mutex);
    }
}

fn on_query(state: &ZenohClockState, query: Query) {
    let payload = match query.payload() {
        Some(p) => p,
        None => { reply_error(query, 2); return; }
    };

    if payload.len() < core::mem::size_of::<ClockAdvanceReq>() {
        reply_error(query, 2);
        return;
    }

    let bytes = payload.to_bytes();
    let req: ClockAdvanceReq = unsafe { ptr::read_unaligned(bytes.as_ptr() as *const _) };
    
    state.delta_ns.store(req.delta_ns as i64, Ordering::Release);
    state.mujoco_time_ns.store(req.mujoco_time_ns as i64, Ordering::Release);

    unsafe {
        virtmcu_mutex_lock(state.mutex);
        (*state.inner).quantum_ready = true;
        virtmcu_cond_signal(state.vcpu_cond);

        let mut error_code = 0;
        while !(*state.inner).quantum_done {
            let rc = virtmcu_cond_timedwait(state.query_cond, state.mutex, 60000);
            if rc == 0 && !(*state.inner).quantum_done {
                error_code = 1; // STALL
                break;
            }
        }


        let vtime = (*state.inner).vtime_ns;
        virtmcu_mutex_unlock(state.mutex);

        let resp = ClockReadyResp {
            current_vtime_ns: vtime as u64,
            n_frames: 0,
            error_code,
        };

        let resp_bytes: &[u8] = core::slice::from_raw_parts(
            &resp as *const _ as *const u8,
            core::mem::size_of::<ClockReadyResp>(),
        );

        let _ = query.reply(query.key_expr(), ZBytes::from(resp_bytes)).wait();
    }
}

fn reply_error(query: Query, error_code: u32) {
    let resp = ClockReadyResp { current_vtime_ns: 0, n_frames: 0, error_code };
    let resp_bytes: &[u8] = unsafe {
        core::slice::from_raw_parts(&resp as *const _ as *const u8, core::mem::size_of::<ClockReadyResp>())
    };
    let _ = query.reply(query.key_expr(), ZBytes::from(resp_bytes)).wait();
}

extern "C" {
    fn virtmcu_icount_advance(delta: i64);
}
