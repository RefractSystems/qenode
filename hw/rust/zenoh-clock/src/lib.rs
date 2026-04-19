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
    clippy::while_immutable_condition
)]

use core::ffi::{c_char, c_void};
use std::ffi::{CStr, CString};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use virtmcu_qom::error::Error;
use virtmcu_qom::qdev::{DeviceClass, SysBusDevice};
use virtmcu_qom::qom::{Object, ObjectClass, TypeInfo};
use virtmcu_qom::timer::{
    qemu_clock_get_ns, virtmcu_timer_free, virtmcu_timer_mod, virtmcu_timer_new_ns, QemuTimer,
    QEMU_CLOCK_VIRTUAL,
};
use virtmcu_qom::{
    declare_device_type, define_prop_string, define_prop_uint32, define_properties, device_class,
    device_class_set_props, vlog,
};
use zenoh::query::Query;
use zenoh::Session;
use zenoh::Wait;

/* ── FFI Types ────────────────────────────────────────────────────────────── */

#[repr(C)]
pub struct CPUState {
    _opaque: [u8; 0],
}

extern "C" {
    pub fn virtmcu_cpu_set_halt_hook(cb: Option<extern "C" fn(*mut CPUState, bool)>);
    pub fn virtmcu_cpu_set_tcg_hook(cb: Option<extern "C" fn(*mut CPUState)>);
}

/* ── QOM Object ───────────────────────────────────────────────────────────── */

#[repr(C)]
pub struct ZenohClock {
    pub parent_obj: SysBusDevice,

    /* Properties */
    pub node_id: u32,
    pub mode: *mut c_char,
    pub router: *mut c_char,
    pub stall_timeout: u32,

    /* Internal State */
    pub next_quantum_ns: i64,
    pub last_halt_vtime: i64,
    pub quantum_timer: *mut QemuTimer,

    /* Rust state */
    pub rust_state: *mut ZenohClockBackend,
}

pub struct ZenohClockBackend {
    session: Session,
    node_id: u32,
    stall_timeout_ms: u32,

    /* Communication state */
    mutex: Mutex<()>,
    cond: Condvar,

    quantum_ready: AtomicBool,
    quantum_done: AtomicBool,
    delta_ns: AtomicU64,
    vtime_ns: AtomicU64,
    mujoco_time_ns: AtomicU64,
}

/* ── Logic ────────────────────────────────────────────────────────────────── */

static mut GLOBAL_CLOCK: *mut ZenohClock = ptr::null_mut();

extern "C" fn zenoh_clock_quantum_timer_cb(opaque: *mut c_void) {
    zenoh_clock_cpu_halt_cb(ptr::null_mut(), false);
}

extern "C" fn zenoh_clock_cpu_tcg_hook(_cpu: *mut CPUState) {
    zenoh_clock_cpu_halt_cb(_cpu, false);
}

extern "C" fn zenoh_clock_cpu_halt_cb(_cpu: *mut CPUState, halted: bool) {
    let s_ptr = unsafe { GLOBAL_CLOCK };
    if s_ptr.is_null() {
        return;
    }
    let s = unsafe { &mut *s_ptr };
    if s.rust_state.is_null() {
        return;
    }

    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };

    // In slaved mode, we ONLY block when we reach the virtual time boundary.
    // This handles both instruction execution and WFI (where
    // virtual time advances via host clock in suspend mode).
    let should_block = now >= s.next_quantum_ns;

    if should_block {
        let backend = unsafe { &*s.rust_state };

        // Release BQL before blocking
        let was_locked = unsafe { virtmcu_qom::sync::virtmcu_bql_locked() };
        if was_locked {
            unsafe { virtmcu_qom::sync::Bql::unlock() };
        }

        let delta = zenoh_clock_quantum_wait_internal(backend, now as u64);

        if was_locked {
            let _bql = virtmcu_qom::sync::Bql::lock();
            std::mem::forget(_bql);
        }

        // 1. Advance virtual clock manually if requested by TA.
        // This ensures that 'suspend' mode advances and 'icount' mode wakes up from WFI.
        let target_vtime = s.next_quantum_ns + delta as i64;
        let now_after_block = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };

        if delta > 0 {
            let should_advance = !virtmcu_qom::icount::icount_enabled() || halted;
            if should_advance && target_vtime > now_after_block {
                virtmcu_qom::icount::icount_advance(target_vtime - now_after_block);
            }
        }

        // 2. Set next boundary
        s.next_quantum_ns = target_vtime;

        // Final safety: ensure it's always in the future relative to final time
        let now_final = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };
        if s.next_quantum_ns <= now_final {
            s.next_quantum_ns = now_final + 1;
        }

        if !s.quantum_timer.is_null() {
            unsafe {
                virtmcu_timer_mod(s.quantum_timer, s.next_quantum_ns);
            }
        }
    }
}

fn zenoh_clock_quantum_wait_internal(backend: &ZenohClockBackend, _vtime_ns: u64) -> u64 {
    backend.vtime_ns.store(_vtime_ns, Ordering::SeqCst);
    backend.quantum_done.store(true, Ordering::SeqCst);

    {
        let _guard = backend.mutex.lock().unwrap();
        backend.cond.notify_all();
    }

    let start = Instant::now();
    let timeout = Duration::from_millis(backend.stall_timeout_ms as u64);

    let mut guard = backend.mutex.lock().unwrap();
    while !backend.quantum_ready.load(Ordering::SeqCst) {
        let (new_guard, result) = backend
            .cond
            .wait_timeout(guard, Duration::from_millis(100))
            .unwrap();
        guard = new_guard;
        if result.timed_out() && start.elapsed() > timeout {
            break;
        }
    }

    backend.quantum_ready.store(false, Ordering::SeqCst);
    backend.delta_ns.load(Ordering::SeqCst)
}

fn on_clock_query(backend: &ZenohClockBackend, query: Query) {
    let payload = match query.payload() {
        Some(p) => p,
        None => return,
    };

    if payload.len() < 16 {
        return;
    }

    let payload_bytes = payload.to_bytes();
    let delta = u64::from_le_bytes(payload_bytes[0..8].try_into().unwrap());
    let mujoco = u64::from_le_bytes(payload_bytes[8..16].try_into().unwrap());

    let start = Instant::now();
    let timeout = Duration::from_millis(backend.stall_timeout_ms as u64);

    let mut guard = backend.mutex.lock().unwrap();
    while !backend.quantum_done.load(Ordering::SeqCst) {
        let (new_guard, result) = backend
            .cond
            .wait_timeout(guard, Duration::from_millis(100))
            .unwrap();
        guard = new_guard;
        if result.timed_out() && start.elapsed() > timeout {
            return;
        }
    }

    backend.quantum_done.store(false, Ordering::SeqCst);
    let reached_vtime = backend.vtime_ns.load(Ordering::SeqCst);

    backend.delta_ns.store(delta, Ordering::SeqCst);
    backend.mujoco_time_ns.store(mujoco, Ordering::SeqCst);
    backend.quantum_ready.store(true, Ordering::SeqCst);
    backend.cond.notify_all();

    drop(guard);

    #[repr(C)]
    struct ClockReadyResp {
        vtime_ns: u64,
        n_frames: u32,
        error_code: u32,
    }

    let resp = ClockReadyResp {
        vtime_ns: reached_vtime,
        n_frames: 0,
        error_code: 0,
    };

    let mut resp_bytes = [0u8; 16];
    unsafe {
        ptr::copy_nonoverlapping(
            &resp as *const ClockReadyResp as *const u8,
            resp_bytes.as_mut_ptr(),
            16,
        );
    }

    std::thread::spawn(move || {
        let _ = query.reply(query.key_expr(), resp_bytes.as_slice()).wait();
    });
}

/* ── Boilerplate ──────────────────────────────────────────────────────────── */

unsafe extern "C" fn zenoh_clock_realize(dev: *mut c_void, errp: *mut *mut c_void) {
    let s = &mut *(dev as *mut ZenohClock);

    let mode_str = if s.mode.is_null() {
        "slaved-suspend"
    } else {
        CStr::from_ptr(s.mode).to_str().unwrap_or("slaved-suspend")
    };

    if mode_str != "icount" && mode_str != "suspend" && mode_str != "slaved-suspend" {
        return;
    }

    let router_str = if s.router.is_null() {
        ptr::null()
    } else {
        s.router as *const c_char
    };

    let stall_ms = if s.stall_timeout == 0 {
        5000
    } else {
        s.stall_timeout
    };

    s.rust_state = zenoh_clock_init_internal(s.node_id, router_str, stall_ms);

    if s.rust_state.is_null() {
        virtmcu_qom::error_setg!(
            errp as *mut *mut Error,
            c"zenoh-clock: failed to initialize Rust backend".as_ptr()
        );
        return;
    }

    s.next_quantum_ns = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };
    s.last_halt_vtime = -1;
    s.quantum_timer = unsafe {
        virtmcu_timer_new_ns(
            QEMU_CLOCK_VIRTUAL,
            zenoh_clock_quantum_timer_cb,
            dev as *mut c_void,
        )
    };

    unsafe {
        GLOBAL_CLOCK = s;
        virtmcu_cpu_set_halt_hook(Some(zenoh_clock_cpu_halt_cb));
        virtmcu_cpu_set_tcg_hook(Some(zenoh_clock_cpu_tcg_hook));
    }

    vlog!(
        "[zenoh-clock] Realized (mode={}, node={}, stall_timeout={} ms)\n",
        mode_str,
        s.node_id,
        stall_ms
    );
}

unsafe extern "C" fn zenoh_clock_instance_finalize(obj: *mut Object) {
    let s = &mut *(obj as *mut ZenohClock);
    if !s.rust_state.is_null() {
        unsafe {
            drop(Box::from_raw(s.rust_state));
        }
        s.rust_state = ptr::null_mut();
    }
    if !s.quantum_timer.is_null() {
        unsafe {
            virtmcu_timer_free(s.quantum_timer);
        }
        s.quantum_timer = ptr::null_mut();
    }
    unsafe {
        virtmcu_cpu_set_halt_hook(None);
        virtmcu_cpu_set_tcg_hook(None);
        GLOBAL_CLOCK = ptr::null_mut();
    }
}

unsafe extern "C" fn zenoh_clock_instance_init(obj: *mut Object) {
    let s = &mut *(obj as *mut ZenohClock);
    s.rust_state = ptr::null_mut();
    s.quantum_timer = ptr::null_mut();
}

define_properties!(
    ZENOH_CLOCK_PROPERTIES,
    [
        define_prop_uint32!(c"node".as_ptr(), ZenohClock, node_id, 0),
        define_prop_string!(c"mode".as_ptr(), ZenohClock, mode),
        define_prop_string!(c"router".as_ptr(), ZenohClock, router),
        define_prop_uint32!(c"stall-timeout".as_ptr(), ZenohClock, stall_timeout, 5000),
    ]
);

unsafe extern "C" fn zenoh_clock_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let dc = device_class!(klass);
    unsafe {
        (*dc).realize = Some(zenoh_clock_realize);
        (*dc).user_creatable = true;
    }
    virtmcu_qom::device_class_set_props!(dc, ZENOH_CLOCK_PROPERTIES);
}

static ZENOH_CLOCK_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"zenoh-clock".as_ptr(),
    parent: c"sys-bus-device".as_ptr(),
    instance_size: std::mem::size_of::<ZenohClock>(),
    instance_align: 0,
    instance_init: Some(zenoh_clock_instance_init),
    instance_post_init: None,
    instance_finalize: Some(zenoh_clock_instance_finalize),
    abstract_: false,
    class_size: 0,
    class_init: Some(zenoh_clock_class_init),
    class_base_init: None,
    class_data: ptr::null(),
    interfaces: ptr::null(),
};

declare_device_type!(zenoh_clock_type_init, ZENOH_CLOCK_TYPE_INFO);

/* ── Internal Rust State ─────────────────────────────────────────────────── */

fn zenoh_clock_init_internal(
    node_id: u32,
    router: *const c_char,
    stall_timeout_ms: u32,
) -> *mut ZenohClockBackend {
    let session = unsafe {
        match virtmcu_zenoh::open_session(router) {
            Ok(s) => s,
            Err(e) => {
                vlog!("[zenoh-clock] failed to open Zenoh session: {:?}\n", e);
                return ptr::null_mut();
            }
        }
    };

    let backend = Box::new(ZenohClockBackend {
        session: session.clone(),
        node_id,
        stall_timeout_ms,
        mutex: Mutex::new(()),
        cond: Condvar::new(),
        quantum_ready: AtomicBool::new(false),
        quantum_done: AtomicBool::new(false),
        delta_ns: AtomicU64::new(0),
        vtime_ns: AtomicU64::new(0),
        mujoco_time_ns: AtomicU64::new(0),
    });

    let backend_ptr = Box::into_raw(backend) as usize;
    let topic = format!("sim/clock/advance/{}", node_id);

    let queryable = match session
        .declare_queryable(topic)
        .callback(move |query| {
            let backend = unsafe { &*(backend_ptr as *const ZenohClockBackend) };
            on_clock_query(backend, query);
        })
        .wait()
    {
        Ok(q) => q,
        Err(e) => {
            eprintln!("zenoh-clock: failed to declare queryable: {:?}", e);
            return ptr::null_mut();
        }
    };

    std::mem::forget(queryable);

    // Heartbeat thread
    let hb_session = session.clone();
    let node_id_hb = node_id;
    std::thread::spawn(move || loop {
        let topic = format!("sim/clock/heartbeat/{}", node_id_hb);
        let _ = hb_session.put(topic, vec![1]).wait();
        std::thread::sleep(Duration::from_millis(1000));
    });

    backend_ptr as *mut ZenohClockBackend
}
