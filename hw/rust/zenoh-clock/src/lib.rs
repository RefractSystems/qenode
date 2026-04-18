#![allow(
    clippy::missing_safety_doc,
    clippy::collapsible_match,
    dead_code,
    unused_imports,
    clippy::len_zero,
    clippy::while_immutable_condition
)]
extern crate libc;

use core::ffi::{c_char, c_void};
use std::ffi::{CStr, CString};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use virtmcu_api::{ClockAdvanceReq, ClockReadyResp};
use virtmcu_qom::cpu::{virtmcu_cpu_exit_all, CPUState, virtmcu_cpu_halt_hook, virtmcu_tcg_quantum_hook};
use virtmcu_qom::error::{Error, error_setg};
use virtmcu_qom::icount::icount_enabled;
use virtmcu_qom::qdev::{DeviceClass, SysBusDevice, device_class_set_props};
use virtmcu_qom::qom::{ObjectClass, TypeInfo, Object};
use virtmcu_qom::sync::{
    virtmcu_cond_signal, virtmcu_cond_timedwait, virtmcu_cond_wait, virtmcu_mutex_lock,
    virtmcu_mutex_unlock, QemuCond, QemuMutex, Bql,
};
use virtmcu_qom::timer::{QEMU_CLOCK_VIRTUAL, QemuTimer, qemu_clock_get_ns, virtmcu_timer_free, virtmcu_timer_mod, virtmcu_timer_new_ns};
use virtmcu_qom::{declare_device_type, device_class, define_prop_uint32, define_prop_string, define_properties};
use zenoh::query::{Query, Queryable};
use zenoh::{Config, Session, Wait};

#[repr(C)]
pub struct ZenohClock {
    pub parent_obj: SysBusDevice,
    pub node_id: u32,
    pub router: *mut c_char,
    pub mode: *mut c_char,
    pub stall_timeout_ms: u32,

    pub rust_state: *mut ZenohClockBackend,
    pub mutex: QemuMutex,
    pub vcpu_cond: QemuCond,
    pub query_cond: QemuCond,
    pub next_quantum_ns: i64,
    pub quantum_timer: *mut QemuTimer,
}

pub struct ZenohClockBackend {
    session: Session,
    queryable: Option<Queryable<()>>,
    node_id: u32,
    stall_timeout_ms: u32,
    mutex: *mut QemuMutex,
    vcpu_cond: *mut QemuCond,
    query_cond: *mut QemuCond,
    delta_ns: AtomicI64,
    mujoco_time_ns: AtomicI64,
    query_ready: AtomicBool,
}

unsafe impl Send for ZenohClockBackend {}
unsafe impl Sync for ZenohClockBackend {}

static mut GLOBAL_CLOCK: *mut ZenohClock = ptr::null_mut();

extern "C" fn zenoh_clock_timer_cb(_opaque: *mut c_void) {
    unsafe {
        virtmcu_cpu_exit_all();
    }
}

extern "C" fn zenoh_clock_cpu_halt_cb(_cpu: *mut CPUState, halted: bool) {
    let s = unsafe { &mut *GLOBAL_CLOCK };
    if s.rust_state.is_null() {
        return;
    }

    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };
    if now >= s.next_quantum_ns || halted {
        let backend = unsafe { &*s.rust_state };
        
        let mut bql = Bql::lock();
        // Release BQL before blocking
        drop(bql);

        let delta = zenoh_clock_quantum_wait_internal(backend, now);

        bql = Bql::lock();

        s.next_quantum_ns = now + delta;
        if !s.quantum_timer.is_null() {
            unsafe {
                virtmcu_timer_mod(s.quantum_timer, s.next_quantum_ns);
            }
        }
        // Keep BQL held when returning to QEMU
        std::mem::forget(bql);
    }
}

extern "C" fn zenoh_clock_tcg_quantum_cb(cpu: *mut CPUState) {
    zenoh_clock_cpu_halt_cb(cpu, false);
}

unsafe extern "C" fn zenoh_clock_realize(dev: *mut c_void, errp: *mut *mut Error) {
    let s = &mut *(dev as *mut ZenohClock);

    if !GLOBAL_CLOCK.is_null() {
        error_setg!(errp, c"Only one zenoh-clock instance is supported".as_ptr());
        return;
    }

    unsafe {
        virtmcu_qom::sync::qemu_mutex_init(&mut s.mutex);
        virtmcu_qom::sync::qemu_cond_init(&mut s.vcpu_cond);
        virtmcu_qom::sync::qemu_cond_init(&mut s.query_cond);
    }
    s.next_quantum_ns = 0;

    if !icount_enabled() {
        s.quantum_timer = unsafe {
            virtmcu_timer_new_ns(QEMU_CLOCK_VIRTUAL, zenoh_clock_timer_cb, dev)
        };
    } else {
        s.quantum_timer = ptr::null_mut();
    }

    let mut stall_ms = s.stall_timeout_ms;
    if stall_ms == 0 {
        if let Ok(env_val) = std::env::var("VIRTMCU_STALL_TIMEOUT_MS") {
            if let Ok(val) = env_val.parse::<u32>() {
                stall_ms = val;
            }
        }
        if stall_ms == 0 {
            stall_ms = 5000;
        }
    }

    let router_str = if s.router.is_null() {
        ptr::null()
    } else {
        s.router as *const c_char
    };

    s.rust_state = zenoh_clock_init_internal(s.node_id, router_str, stall_ms,
                                             &mut s.mutex, &mut s.vcpu_cond, &mut s.query_cond);
    
    if s.rust_state.is_null() {
        error_setg!(errp, c"zenoh-clock: failed to initialize Rust backend".as_ptr());
        return;
    }

    unsafe {
        GLOBAL_CLOCK = s;
        virtmcu_cpu_halt_hook = Some(zenoh_clock_cpu_halt_cb);
        virtmcu_tcg_quantum_hook = Some(zenoh_clock_tcg_quantum_cb);
    }
}

unsafe extern "C" fn zenoh_clock_instance_finalize(obj: *mut Object) {
    let s = &mut *(obj as *mut ZenohClock);
    if s as *mut ZenohClock == GLOBAL_CLOCK {
        virtmcu_cpu_halt_hook = None;
        virtmcu_tcg_quantum_hook = None;
        GLOBAL_CLOCK = ptr::null_mut();
    }
    if !s.rust_state.is_null() {
        zenoh_clock_free_internal(s.rust_state);
        s.rust_state = ptr::null_mut();
    }
    if !s.quantum_timer.is_null() {
        virtmcu_timer_free(s.quantum_timer);
        s.quantum_timer = ptr::null_mut();
    }
    virtmcu_qom::sync::qemu_mutex_destroy(&mut s.mutex);
    virtmcu_qom::sync::qemu_cond_destroy(&mut s.vcpu_cond);
    virtmcu_qom::sync::qemu_cond_destroy(&mut s.query_cond);
}

define_properties!(ZENOH_CLOCK_PROPERTIES, [
    define_prop_uint32!(c"node".as_ptr(), ZenohClock, node_id, 0),
    define_prop_string!(c"router".as_ptr(), ZenohClock, router),
    define_prop_string!(c"mode".as_ptr(), ZenohClock, mode),
    define_prop_uint32!(c"stall-timeout".as_ptr(), ZenohClock, stall_timeout_ms, 0),
]);

unsafe extern "C" fn zenoh_clock_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let dc = device_class!(klass);
    unsafe {
        (*dc).realize = Some(zenoh_clock_realize);
        (*dc).user_creatable = true;
        device_class_set_props(dc, ZENOH_CLOCK_PROPERTIES.as_ptr());
    }
}

static ZENOH_CLOCK_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"zenoh-clock".as_ptr(),
    parent: c"sys-bus-device".as_ptr(),
    instance_size: std::mem::size_of::<ZenohClock>(),
    instance_align: 0,
    instance_init: None,
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

/* ── Backend Logic (formerly in lib.rs) ────────────────────────────────────── */

fn zenoh_clock_init_internal(
    node_id: u32,
    router: *const c_char,
    stall_timeout_ms: u32,
    mutex: *mut QemuMutex,
    vcpu_cond: *mut QemuCond,
    query_cond: *mut QemuCond,
) -> *mut ZenohClockBackend {
    let session = match virtmcu_zenoh::open_session(router) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("failed to open Zenoh session: {:?}", e);
            return ptr::null_mut();
        }
    };

    let backend = Arc::new(ZenohClockBackend {
        session: session.clone(),
        queryable: None,
        node_id,
        stall_timeout_ms,
        mutex,
        vcpu_cond,
        query_cond,
        delta_ns: AtomicI64::new(0),
        mujoco_time_ns: AtomicI64::new(0),
        query_ready: AtomicBool::new(false),
    });

    let backend_ptr = Arc::as_ptr(&backend) as usize;
    let topic = format!("sim/clock/advance/{}", node_id);

    let queryable = session
        .declare_queryable(topic)
        .callback(move |query| {
            let backend = unsafe { &*(backend_ptr as *const ZenohClockBackend) };
            on_clock_query(backend, query);
        })
        .wait()
        .unwrap();

    let mut backend_mut = Arc::try_unwrap(backend).ok().unwrap();
    backend_mut.queryable = Some(queryable);

    Box::into_raw(Box::new(backend_mut))
}

fn zenoh_clock_free_internal(backend: *mut ZenohClockBackend) {
    if !backend.is_null() {
        unsafe {
            drop(Box::from_raw(backend));
        }
    }
}

fn zenoh_clock_quantum_wait_internal(backend: &ZenohClockBackend, _vtime_ns: u64) -> i64 {
    unsafe {
        virtmcu_mutex_lock(backend.mutex);
        while !backend.query_ready.load(Ordering::Acquire) {
            virtmcu_cond_wait(backend.vcpu_cond, backend.mutex);
        }
        backend.query_ready.store(false, Ordering::Release);
        virtmcu_mutex_unlock(backend.mutex);
    }
    backend.delta_ns.load(Ordering::Acquire)
}

fn on_clock_query(backend: &ZenohClockBackend, query: Query) {
    let payload = match query.payload() {
        Some(p) => p,
        None => return,
    };

    if payload.len() < 16 {
        return;
    }

    let mut delta: u64 = 0;
    let mut mujoco: u64 = 0;
    let data = payload.to_bytes();
    unsafe {
        ptr::copy_nonoverlapping(data.as_ptr(), &mut delta as *mut u64 as *mut u8, 8);
        ptr::copy_nonoverlapping(data.as_ptr().add(8), &mut mujoco as *mut u64 as *mut u8, 8);
    }

    backend.delta_ns.store(delta as i64, Ordering::Release);
    backend
        .mujoco_time_ns
        .store(mujoco as i64, Ordering::Release);

    unsafe {
        virtmcu_mutex_lock(backend.mutex);
        backend.query_ready.store(true, Ordering::Release);
        virtmcu_cond_signal(backend.vcpu_cond);

        while backend.query_ready.load(Ordering::Acquire) {
            if virtmcu_cond_timedwait(
                backend.query_cond,
                backend.mutex,
                backend.stall_timeout_ms,
            ) != 0
            {
                // Stall detected
                break;
            }
        }
        virtmcu_mutex_unlock(backend.mutex);
    }

    let resp = ClockReadyResp {
        current_vtime_ns: 0,
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

    query.reply(query.key_expr(), resp_bytes.as_slice()).wait().unwrap();
}
