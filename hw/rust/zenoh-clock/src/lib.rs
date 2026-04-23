//! Zenoh-based deterministic clock for VirtMCU nodes.
//!
//! This module provides the `ZenohClock` QOM device, which synchronizes
//! the guest's virtual time with an external TimeAuthority via Zenoh.

use core::ffi::{c_char, c_void};
use crossbeam_channel::{Receiver, Sender};
use std::ffi::CStr;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak};

static ACTIVE_HOOKS: AtomicUsize = AtomicUsize::new(0);
use std::time::{Duration, Instant};
use virtmcu_api::{ClockAdvanceReq, ClockReadyResp, CLOCK_ERROR_OK};
use virtmcu_qom::qdev::SysBusDevice;
use virtmcu_qom::qom::{Object, ObjectClass, TypeInfo};
use virtmcu_qom::timer::{
    qemu_clock_get_ns, virtmcu_timer_free, virtmcu_timer_mod, virtmcu_timer_new_ns, QemuTimer,
    QEMU_CLOCK_VIRTUAL,
};
use virtmcu_qom::{
    declare_device_type, define_prop_string, define_prop_uint32, define_properties, device_class,
    vlog,
};
use zenoh::query::{Query, Queryable};
use zenoh::Session;
use zenoh::Wait;

/* ── FFI Types ────────────────────────────────────────────────────────────── */

/// Opaque handle to a QEMU CPUState.
use virtmcu_qom::cpu::CPUState;

/* ── QOM Object ───────────────────────────────────────────────────────────── */

/// Zenoh-synchronized clock device.
#[repr(C)]
pub struct ZenohClock {
    /// Parent object.
    pub parent_obj: SysBusDevice,

    /* Properties */
    /// Unique node ID for clock synchronization.
    pub node_id: u32,
    /// Synchronization mode ("slaved-suspend" or "slaved-icount").
    pub mode: *mut c_char,
    /// Optional Zenoh router address.
    pub router: *mut c_char,
    /// Timeout in milliseconds before a clock stall is declared.
    pub stall_timeout: u32,

    /* Internal State */
    /// Virtual time (ns) of the next quantum boundary.
    pub next_quantum_ns: i64,
    /// Virtual time (ns) of the last halt event.
    pub last_halt_vtime: i64,
    /// Timer used to trigger quantum boundary checks.
    pub quantum_timer: *mut QemuTimer,

    /* Rust state */
    /// Opaque pointer to the Rust backend state.
    pub rust_state: *mut ZenohClockBackend,
}

/// Internal Rust backend for `ZenohClock`.
pub struct ZenohClockBackend {
    /// Zenoh session for communication.
    pub session: Session,
    /// Unique node ID.
    pub node_id: u32,
    /// Stall timeout in milliseconds.
    pub stall_timeout_ms: u32,

    /* Communication state */
    /// Mutex for protecting communication state.
    pub mutex: Mutex<()>,
    /// Condvar for signaling quantum events.
    pub cond: Condvar,

    /// Whether the next quantum is ready to run.
    pub quantum_ready: AtomicBool,
    /// Whether the current quantum has finished.
    pub quantum_done: AtomicBool,
    /// Number of nanoseconds to advance in the current/next quantum.
    pub delta_ns: AtomicU64,
    /// Current virtual time in nanoseconds as known by the backend.
    pub vtime_ns: AtomicU64,
    /// Absolute simulation time in nanoseconds as reported by TimeAuthority.
    pub mujoco_time_ns: AtomicU64,
    /// Cumulative count of clock stalls.
    pub stall_count: AtomicU64,

    /// Zenoh queryable for clock advance requests.
    pub queryable: Option<Queryable<()>>,

    /// Channel for sending clock queries to the worker thread.
    pub query_sender: Option<Sender<Query>>,

    /* Profiling state */
    /// Total time spent waiting for the Big QEMU Lock (BQL).
    pub total_bql_wait_ns: AtomicU64,
    /// Total number of quantum iterations that acquired BQL.
    pub total_iterations: AtomicU64,
    /// Total number of quantum iterations that did NOT acquire BQL.
    pub total_no_bql_iterations: AtomicU64,
    /// Last time a performance report was generated.
    pub last_report_time: Mutex<Instant>,
    /// Time when the backend was initialized.
    pub start_time: Instant,

    /* Lifecycle */
    /// Whether the backend is shutting down.
    pub shutdown: Arc<AtomicBool>,
}

/* ── Logic ────────────────────────────────────────────────────────────────── */

static GLOBAL_CLOCK: AtomicPtr<ZenohClock> = AtomicPtr::new(ptr::null_mut());

extern "C" fn zenoh_clock_quantum_timer_cb(_opaque: *mut c_void) {
    zenoh_clock_cpu_halt_cb(ptr::null_mut(), false);
}

extern "C" fn zenoh_clock_cpu_tcg_hook(_cpu: *mut CPUState) {
    zenoh_clock_cpu_halt_cb(_cpu, false);
}

extern "C" fn zenoh_clock_cpu_halt_cb(_cpu: *mut CPUState, halted: bool) {
    // 1. Signal that we are entering a hook
    ACTIVE_HOOKS.fetch_add(1, Ordering::SeqCst);

    // 2. Check if the clock device is still alive.
    let s_ptr = GLOBAL_CLOCK.load(Ordering::Acquire);
    if !s_ptr.is_null() {
        let s = unsafe { &mut *s_ptr };
        if !s.rust_state.is_null() {
            zenoh_clock_cpu_halt_cb_internal(s, _cpu, halted);
        }
    }

    // 3. Signal that we have finished.
    ACTIVE_HOOKS.fetch_sub(1, Ordering::SeqCst);
}

fn zenoh_clock_cpu_halt_cb_internal(s: &mut ZenohClock, _cpu: *mut CPUState, halted: bool) {
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

        let raw_delta = zenoh_clock_quantum_wait_internal(backend, now as u64);
        // On stall the sentinel is returned; treat as zero advance (hold position).
        let delta = if raw_delta == QUANTUM_WAIT_STALL_SENTINEL { 0 } else { raw_delta };

        if was_locked {
            let bql_start = Instant::now();
            virtmcu_qom::sync::Bql::lock_forget();
            let bql_wait = bql_start.elapsed().as_nanos() as u64;
            backend.total_bql_wait_ns.fetch_add(bql_wait, Ordering::Relaxed);
            backend.total_iterations.fetch_add(1, Ordering::Relaxed);
        } else {
            backend.total_no_bql_iterations.fetch_add(1, Ordering::Relaxed);
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

        // Final safety: ensure it's always in the future relative to final time.
        // We only add 1 if we're NOT advancing (delta=0) to prevent immediate re-blocking.
        let now_final = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };
        if s.next_quantum_ns <= now_final {
            s.next_quantum_ns = now_final + i64::from(delta == 0);
        }

        if !s.quantum_timer.is_null() {
            unsafe {
                virtmcu_timer_mod(s.quantum_timer, s.next_quantum_ns);
            }
        }
    }
}

/// Return value of `zenoh_clock_quantum_wait_internal`: delta_ns on success,
/// or `u64::MAX` as a sentinel indicating a stall timeout.
const QUANTUM_WAIT_STALL_SENTINEL: u64 = u64::MAX;

fn zenoh_clock_quantum_wait_internal(backend: &ZenohClockBackend, _vtime_ns: u64) -> u64 {
    virtmcu_qom::vlog!("[zenoh-clock] quantum_wait at vtime={}\n", _vtime_ns);
    // Runtime assertion (not just debug_assert): BQL must NOT be held here.
    // Violating this causes a deadlock when on_clock_query tries to reply.
    if unsafe { virtmcu_qom::sync::virtmcu_bql_locked() } {
        // We only warn if the VM is actually running. During initialization or
        // teardown (teardown_cpus), hooks might be called with BQL held from
        // the main thread; since sync is bypassed anyway, logging is just noise.
        if virtmcu_qom::sysemu::runstate_is_running() {
            virtmcu_qom::vlog!(
                "[zenoh-clock] WARNING: BQL held entering quantum_wait — would deadlock. Skipping sync.\n"
            );
        }
        return QUANTUM_WAIT_STALL_SENTINEL;
    }

    backend.vtime_ns.store(_vtime_ns, Ordering::SeqCst);
    backend.quantum_done.store(true, Ordering::SeqCst);

    // Notify TA that we finished previous quantum
    {
        let _guard = backend.mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        backend.cond.notify_all();
    }

    let start = Instant::now();
    let timeout = Duration::from_millis(u64::from(backend.stall_timeout_ms));

    // Spin briefly to avoid context switch latency for very fast quantums
    while !backend.quantum_ready.load(Ordering::SeqCst) {
        if start.elapsed() > Duration::from_millis(1) {
            break;
        }
        std::hint::spin_loop();
    }

    if !backend.quantum_ready.load(Ordering::SeqCst) {
        let mut guard = backend.mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        while !backend.quantum_ready.load(Ordering::SeqCst) {
            let (new_guard, result) = backend
                .cond
                .wait_timeout(guard, Duration::from_millis(100))
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard = new_guard;
            if result.timed_out() && start.elapsed() > timeout {
                backend.stall_count.fetch_add(1, Ordering::Relaxed);
                virtmcu_qom::vlog!(
                    "[virtmcu-clock] STALL DETECTED: no clock-advance reply after {} ms (stall #{}). \
                     Reporting to TimeAuthority.\n",
                    backend.stall_timeout_ms,
                    backend.stall_count.load(Ordering::Relaxed)
                );
                return QUANTUM_WAIT_STALL_SENTINEL;
            }
        }
    }

    backend.quantum_ready.store(false, Ordering::SeqCst);
    backend.delta_ns.load(Ordering::SeqCst)
}

#[allow(clippy::too_many_lines)]
fn zenoh_clock_worker_loop(backend: Arc<ZenohClockBackend>, query_receiver: Receiver<Query>) {
    loop {
        if backend.shutdown.load(Ordering::Acquire) {
            break;
        }

        let query = match query_receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(q) => q,
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
        };

        virtmcu_qom::vlog!("[zenoh-clock] processing query for node {}\n", backend.node_id);

        let payload = match query.payload() {
            Some(p) => p,
            None => {
                virtmcu_qom::vlog!("[zenoh-clock] on_clock_query: no payload received!\n");
                continue;
            }
        };

        if payload.len() < 16 {
            virtmcu_qom::vlog!(
                "[zenoh-clock] on_clock_query: payload too short ({} bytes)\n",
                payload.len()
            );
            continue;
        }

        let payload_bytes = payload.to_bytes();
        let req =
            unsafe { std::ptr::read_unaligned(payload_bytes.as_ptr() as *const ClockAdvanceReq) };
        let delta = req.delta_ns;
        let mujoco = req.mujoco_time_ns;

        virtmcu_qom::vlog!("[zenoh-clock] on_clock_query: delta={}, mujoco={}\n", delta, mujoco);

        let start = Instant::now();
        let timeout = Duration::from_millis(u64::from(backend.stall_timeout_ms));

        let start_vtime = backend.vtime_ns.load(Ordering::SeqCst);
        let target_vtime = start_vtime + delta;

        // 1. Prepare for the next quantum
        backend.delta_ns.store(delta, Ordering::SeqCst);
        backend.mujoco_time_ns.store(mujoco, Ordering::SeqCst);

        let mut error_code = CLOCK_ERROR_OK;

        if delta > 0 {
            backend.quantum_done.store(false, Ordering::SeqCst);
            backend.quantum_ready.store(true, Ordering::SeqCst);

            // 2. Wake up the vCPU thread to run the quantum
            {
                let _guard =
                    backend.mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                backend.cond.notify_all();
            }

            // 3. Wait for completion (synchronously in this background thread)
            while !backend.quantum_done.load(Ordering::SeqCst)
                || backend.vtime_ns.load(Ordering::SeqCst) < target_vtime
            {
                if start.elapsed() > Duration::from_millis(1) {
                    if backend.vtime_ns.load(Ordering::SeqCst) >= target_vtime {
                        break;
                    }
                    break; // Fall through to condvar wait
                }
                std::hint::spin_loop();
            }

            if !backend.quantum_done.load(Ordering::SeqCst)
                || backend.vtime_ns.load(Ordering::SeqCst) < target_vtime
            {
                let mut guard =
                    backend.mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                while !backend.quantum_done.load(Ordering::SeqCst)
                    || backend.vtime_ns.load(Ordering::SeqCst) < target_vtime
                {
                    let (new_guard, result) = backend
                        .cond
                        .wait_timeout(guard, Duration::from_millis(100))
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    guard = new_guard;
                    if result.timed_out() && start.elapsed() > timeout {
                        virtmcu_qom::vlog!(
                            "[virtmcu-clock] STALL: QEMU did not reach quantum boundary (target={}) in time (now={}).\n",
                            target_vtime,
                            backend.vtime_ns.load(Ordering::SeqCst)
                        );
                        error_code = 1; // CLOCK_ERROR_STALL
                        break;
                    }
                }
            }
        } else {
            // Delta = 0: Initial sync or heartbeat.
            if !backend.quantum_done.load(Ordering::SeqCst) {
                let mut guard =
                    backend.mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                while !backend.quantum_done.load(Ordering::SeqCst) {
                    let (new_guard, result) = backend
                        .cond
                        .wait_timeout(guard, Duration::from_millis(100))
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    guard = new_guard;
                    if result.timed_out() && start.elapsed() > timeout {
                        virtmcu_qom::vlog!(
                            "[virtmcu-clock] STALL: QEMU did not signal readiness in time.\n"
                        );
                        error_code = 1; // CLOCK_ERROR_STALL
                        break;
                    }
                }
            }
        }

        // 4. Load the virtual time reached and reply
        let reached_vtime = backend.vtime_ns.load(Ordering::SeqCst);
        let resp = ClockReadyResp { current_vtime_ns: reached_vtime, n_frames: 0, error_code };
        let mut resp_bytes = [0u8; 16];
        unsafe {
            ptr::copy_nonoverlapping(&raw const resp as *const u8, resp_bytes.as_mut_ptr(), 16);
        }
        let _ = query.reply(query.key_expr(), resp_bytes.as_slice()).wait();
    }
}

fn on_clock_query(backend_weak: Weak<ZenohClockBackend>, query: Query) {
    if let Some(backend) = backend_weak.upgrade() {
        if let Some(sender) = &backend.query_sender {
            let _ = sender.send(query);
        }
    }
}

/* ── Boilerplate ──────────────────────────────────────────────────────────── */

unsafe extern "C" fn zenoh_clock_realize(dev: *mut c_void, errp: *mut *mut c_void) {
    let s = &mut *(dev as *mut ZenohClock);

    let mode_str = if s.mode.is_null() {
        "slaved-suspend"
    } else {
        CStr::from_ptr(s.mode).to_str().unwrap_or("slaved-suspend")
    };

    if mode_str != "icount"
        && mode_str != "slaved-icount"
        && mode_str != "suspend"
        && mode_str != "slaved-suspend"
    {
        return;
    }

    let router_str = if s.router.is_null() { ptr::null() } else { s.router.cast_const() };

    let mut stall_ms = s.stall_timeout;
    if stall_ms == 0 {
        if let Ok(val) = std::env::var("VIRTMCU_STALL_TIMEOUT_MS") {
            if let Ok(parsed) = val.parse::<u32>() {
                stall_ms = parsed;
            }
        }
    }
    if stall_ms == 0 {
        stall_ms = 5000;
    }

    s.rust_state = zenoh_clock_init_internal(s.node_id, router_str, stall_ms);

    if s.rust_state.is_null() {
        virtmcu_qom::error_setg!(errp, "zenoh-clock: failed to initialize Rust backend");
        return;
    }

    s.next_quantum_ns = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };
    s.last_halt_vtime = -1;
    s.quantum_timer =
        unsafe { virtmcu_timer_new_ns(QEMU_CLOCK_VIRTUAL, zenoh_clock_quantum_timer_cb, dev) };

    // Task 27.2: Ensure the timer is scheduled initially so we reach the first hook
    // even if the guest is idling or slow to boot.
    unsafe {
        virtmcu_timer_mod(s.quantum_timer, s.next_quantum_ns);
    }

    // Fail loudly if multiple clock devices are instantiated
    let prev = GLOBAL_CLOCK.swap(s, Ordering::AcqRel);
    if !prev.is_null() {
        vlog!("[zenoh-clock] FATAL: Multiple ZenohClock instances realized! VirtMCU supports only one clock authority.\n");
        std::process::abort();
    }

    unsafe {
        virtmcu_qom::cpu::virtmcu_cpu_halt_hook = Some(zenoh_clock_cpu_halt_cb);
        virtmcu_qom::cpu::virtmcu_tcg_quantum_hook = Some(zenoh_clock_cpu_tcg_hook);
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

    // 1. Immediately disable hooks globally and clear the pointer.
    GLOBAL_CLOCK.store(ptr::null_mut(), Ordering::Release);
    unsafe {
        virtmcu_qom::cpu::virtmcu_cpu_halt_hook = None;
        virtmcu_qom::cpu::virtmcu_tcg_quantum_hook = None;
    }

    // 2. Wait for any active hook executions to finish their logic.
    //    Since we set GLOBAL_CLOCK to NULL, any NEW hook entries will return immediately.
    //    This loop waits for those that were already inside.
    let mut attempts = 0;
    while ACTIVE_HOOKS.load(Ordering::SeqCst) > 0 && attempts < 1000 {
        std::thread::yield_now();
        attempts += 1;
    }

    if !s.rust_state.is_null() {
        let backend = unsafe { Arc::from_raw(s.rust_state) };
        // Signal heartbeat thread and worker thread to exit before we free the backend.
        backend.shutdown.store(true, Ordering::Release);

        let total_wait = backend.total_bql_wait_ns.load(Ordering::Relaxed);
        let iterations = backend.total_iterations.load(Ordering::Relaxed);
        let no_bql = backend.total_no_bql_iterations.load(Ordering::Relaxed);
        if iterations > 0 || no_bql > 0 {
            let elapsed = backend.start_time.elapsed().as_secs_f64();
            let avg_wait_us =
                if iterations > 0 { (total_wait as f64 / iterations as f64) / 1000.0 } else { 0.0 };
            let contention = (total_wait as f64 / (elapsed * 1_000_000_000.0)) * 100.0;
            vlog!(
                "[zenoh-clock] FINAL BQL Contention: {:.2}% (avg wait: {:.2} us, samples: {}, no_bql: {})\n",
                contention,
                avg_wait_us,
                iterations,
                no_bql
            );
        }

        // Arc is dropped here
        s.rust_state = ptr::null_mut();
    }
    if !s.quantum_timer.is_null() {
        unsafe {
            virtmcu_timer_free(s.quantum_timer);
        }
        s.quantum_timer = ptr::null_mut();
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
        define_prop_uint32!(c"stall-timeout".as_ptr(), ZenohClock, stall_timeout, 0),
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

#[allow(clippy::too_many_lines)]
fn zenoh_clock_init_internal(
    node_id: u32,
    router: *const c_char,
    stall_timeout_ms: u32,
) -> *mut ZenohClockBackend {
    vlog!("[zenoh-clock] Opening session for node {}...\n", node_id);
    let session = unsafe {
        match virtmcu_zenoh::open_session(router) {
            Ok(s) => s,
            Err(e) => {
                vlog!("[zenoh-clock] failed to open Zenoh session for node {}: {:?}\n", node_id, e);
                return ptr::null_mut();
            }
        }
    };
    vlog!("[zenoh-clock] Session opened for node {}. ID: {}\n", node_id, session.zid());

    let shutdown = Arc::new(AtomicBool::new(false));
    let (query_tx, query_rx) = crossbeam_channel::unbounded();

    let backend = Arc::new(ZenohClockBackend {
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
        stall_count: AtomicU64::new(0),
        queryable: None,
        query_sender: Some(query_tx),
        total_bql_wait_ns: AtomicU64::new(0),
        total_iterations: AtomicU64::new(0),
        total_no_bql_iterations: AtomicU64::new(0),
        last_report_time: Mutex::new(Instant::now()),
        start_time: Instant::now(),
        shutdown: Arc::clone(&shutdown),
    });

    let backend_ptr = Arc::into_raw(Arc::clone(&backend)).cast_mut();
    let backend_weak = Arc::downgrade(&backend);

    // Spawn the worker thread
    let worker_backend = Arc::clone(&backend);
    std::thread::Builder::new()
        .name(format!("zenoh-clock-worker-{node_id}"))
        .spawn(move || {
            zenoh_clock_worker_loop(worker_backend, query_rx);
        })
        .unwrap_or_else(|_| std::process::abort());

    let topic = format!("sim/clock/advance/{node_id}");

    vlog!("[zenoh-clock] Declaring queryable on {}...\n", topic);
    let queryable = match session
        .declare_queryable(topic.clone())
        .callback(move |query| {
            on_clock_query(Weak::clone(&backend_weak), query);
        })
        .wait()
    {
        Ok(q) => q,
        Err(e) => {
            vlog!("[zenoh-clock] failed to declare queryable on {}: {:?}\n", topic, e);
            unsafe { Arc::from_raw(backend_ptr) };
            return ptr::null_mut();
        }
    };
    vlog!("[zenoh-clock] Queryable declared on {}.\n", topic);

    // Safety: we just created the backend, we can safely get a mutable reference
    // to it before it is shared.
    unsafe {
        let mut_backend = &mut *backend_ptr;
        mut_backend.queryable = Some(queryable);
    }

    // Heartbeat thread — exits when backend.shutdown is set by instance_finalize.
    let hb_session = session.clone();
    let node_id_hb = node_id;
    let hb_backend = Arc::clone(&backend);
    let hb_shutdown = Arc::clone(&shutdown);
    std::thread::Builder::new()
        .name(format!("zenoh-clock-hb-{node_id_hb}"))
        .spawn(move || loop {
            if hb_shutdown.load(Ordering::Acquire) {
                break;
            }

            let topic = format!("sim/clock/heartbeat/{node_id_hb}");
            let _ = hb_session.put(topic, vec![1]).wait();

            let backend = hb_backend.as_ref();
            let iterations = backend.total_iterations.load(Ordering::Relaxed);
            let no_bql = backend.total_no_bql_iterations.load(Ordering::Relaxed);
            if iterations > 0 || no_bql > 0 {
                let total_wait = backend.total_bql_wait_ns.load(Ordering::Relaxed);
                let mut last_report = backend
                    .last_report_time
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let elapsed = last_report.elapsed().as_secs_f64();

                if elapsed >= 1.0 {
                    let avg_wait_us = if iterations > 0 {
                        (total_wait as f64 / iterations as f64) / 1000.0
                    } else {
                        0.0
                    };
                    let contention = (total_wait as f64 / (elapsed * 1_000_000_000.0)) * 100.0;

                    vlog!(
                        "[zenoh-clock] BQL Contention: {:.2}% (avg wait: {:.2} us, samples: {}, no_bql: {})\n",
                        contention,
                        avg_wait_us,
                        iterations,
                        no_bql
                    );

                    backend.total_bql_wait_ns.store(0, Ordering::Relaxed);
                    backend.total_iterations.store(0, Ordering::Relaxed);
                    backend.total_no_bql_iterations.store(0, Ordering::Relaxed);
                    *last_report = Instant::now();
                }
            }

            std::thread::sleep(Duration::from_secs(1));
        })
        .unwrap_or_else(|_| std::process::abort()); // "failed to spawn zenoh-clock heartbeat thread");

    backend_ptr
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zenoh_clock_layout() {
        // QOM layout validation
        assert_eq!(
            core::mem::offset_of!(ZenohClock, parent_obj),
            0,
            "SysBusDevice must be the first field"
        );
    }
}
