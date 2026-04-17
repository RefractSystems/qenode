extern crate libc;

use core::ffi::{c_char, c_void};
use std::ffi::{CStr, CString};
use std::ptr;
use std::sync::atomic::{AtomicI64, Ordering};

use zenoh::Config;
use zenoh::Session;
use zenoh::Wait;
use zenoh::query::Query;
use zenoh::query::Queryable;
use zenoh::bytes::ZBytes;

use virtmcu_qom::sync::*;
use virtmcu_qom::timer::*;
use virtmcu_qom::cpu::*;
use virtmcu_qom::proto::*;

// We use a Boxed struct for the state that needs to be shared with Zenoh threads
// and the C wrapper.
pub struct ZenohClockState {
    node_id: u32,
    #[allow(dead_code)]
    is_icount: bool,

    session: Session,
    #[allow(dead_code)]
    queryable: Option<Queryable<()>>,

    quantum_timer: *mut QemuTimer,

    mutex: *mut QemuMutex,
    vcpu_cond: *mut QemuCond,
    query_cond: *mut QemuCond,

    // Atomic fields for lock-free read from SAL/AAL
    delta_ns: AtomicI64,
    mujoco_time_ns: AtomicI64,
    quantum_start_vtime_ns: AtomicI64,

    // These fields are protected by the mutex. 
    // In a pure Rust impl we'd use a Mutex<InnerState>, 
    // but we use QemuMutex for BQL compatibility and FFI.
    // We'll use unsafe blocks to access them.
    inner: *mut ZenohClockInner,
}

struct ZenohClockInner {
    needs_quantum: bool,
    quantum_ready: bool,
    quantum_done: bool,
    vtime_ns: i64,
}

static mut GLOBAL_ZENOH_CLOCK: *mut ZenohClockState = ptr::null_mut();

#[no_mangle]
pub extern "C" fn zenoh_clock_init(
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

    let session = zenoh::open(config).wait().unwrap();

    let is_icount = if !mode.is_null() {
        let mode_str = unsafe { CStr::from_ptr(mode) }.to_str().unwrap();
        mode_str == "icount"
    } else {
        false
    };

    // Allocate QEMU sync primitives
    let mutex = unsafe {
        let m = libc::malloc(core::mem::size_of::<QemuMutex>()) as *mut QemuMutex;
        qemu_mutex_init(m);
        m
    };
    let vcpu_cond = unsafe {
        let c = libc::malloc(core::mem::size_of::<QemuCond>()) as *mut QemuCond;
        qemu_cond_init(c);
        c
    };
    let query_cond = unsafe {
        let c = libc::malloc(core::mem::size_of::<QemuCond>()) as *mut QemuCond;
        qemu_cond_init(c);
        c
    };

    let inner = Box::into_raw(Box::new(ZenohClockInner {
        needs_quantum: true,
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
        inner,
    });

    let state_ptr = Box::into_raw(state_box);
    unsafe { GLOBAL_ZENOH_CLOCK = state_ptr };

    // Declare queryable
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
        (*state_ptr).quantum_timer = timer_new_ns(
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
pub extern "C" fn zenoh_clock_fini(state: *mut ZenohClockState) {
    if state.is_null() {
        return;
    }
    unsafe {
        if GLOBAL_ZENOH_CLOCK == state {
            GLOBAL_ZENOH_CLOCK = ptr::null_mut();
        }
        virtmcu_tcg_quantum_hook = None;
        virtmcu_get_quantum_timing = None;

        let s = Box::from_raw(state);
        if !s.quantum_timer.is_null() {
            timer_free(s.quantum_timer);
        }

        // Drop s will drop session and queryable
        qemu_mutex_destroy(s.mutex);
        qemu_cond_destroy(s.vcpu_cond);
        qemu_cond_destroy(s.query_cond);
        libc::free(s.mutex as *mut libc::c_void);
        libc::free(s.vcpu_cond as *mut libc::c_void);
        libc::free(s.query_cond as *mut libc::c_void);
        
        let _inner = Box::from_raw(s.inner);
    }
}

extern "C" fn zclock_timer_cb(opaque: *mut c_void) {
    let state = unsafe { &*(opaque as *mut ZenohClockState) };
    unsafe {
        qemu_mutex_lock(state.mutex);
        (*state.inner).needs_quantum = true;
        qemu_mutex_unlock(state.mutex);
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
        if GLOBAL_ZENOH_CLOCK.is_null() {
            return;
        }
        &*GLOBAL_ZENOH_CLOCK
    };

    unsafe {
        qemu_mutex_lock(state.mutex);
        if !(*state.inner).needs_quantum {
            qemu_mutex_unlock(state.mutex);
            return;
        }

        // Processing quantum hook
        bql_lock();
        // Check again after BQL if needed, but here we just follow C impl
        (*state.inner).needs_quantum = false;
        (*state.inner).vtime_ns = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL);
        (*state.inner).quantum_done = true;
        qemu_cond_signal(state.query_cond);
        bql_unlock();

        while !(*state.inner).quantum_ready {
            qemu_cond_wait(state.vcpu_cond, state.mutex);
        }

        (*state.inner).quantum_ready = false;
        (*state.inner).quantum_done = false;
        let next_delta = state.delta_ns.load(Ordering::Acquire);
        let vtime = (*state.inner).vtime_ns;
        state.quantum_start_vtime_ns.store(vtime, Ordering::Release);
        qemu_mutex_unlock(state.mutex);

        bql_lock();
        if state.is_icount {
            virtmcu_icount_advance(next_delta);
            qemu_clock_run_all_timers();
        }
        let now = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL);
        timer_mod(state.quantum_timer, now + next_delta);
        bql_unlock();
    }
}

fn on_query(state: &ZenohClockState, query: Query) {
    let payload = match query.payload() {
        Some(p) => p,
        None => {
            reply_error(query, 2);
            return;
        }
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
        qemu_mutex_lock(state.mutex);
        (*state.inner).quantum_done = false;
        (*state.inner).quantum_ready = true;
        qemu_cond_signal(state.query_cond);

        let mut error_code = 0;
        while !(*state.inner).quantum_done {
            if qemu_cond_timedwait(state.query_cond, state.mutex, 10000) != 0 {
                if !(*state.inner).quantum_done {
                    error_code = 1; // STALL
                }
                break;
            }
        }

        let vtime = if error_code == 0 { (*state.inner).vtime_ns } else { 0 };
        qemu_mutex_unlock(state.mutex);

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
    let resp = ClockReadyResp {
        current_vtime_ns: 0,
        n_frames: 0,
        error_code,
    };
    let resp_bytes: &[u8] = unsafe {
        core::slice::from_raw_parts(
            &resp as *const _ as *const u8,
            core::mem::size_of::<ClockReadyResp>(),
        )
    };
    let _ = query.reply(query.key_expr(), ZBytes::from(resp_bytes)).wait();
}

extern "C" {
    fn virtmcu_icount_advance(delta: i64);
}
