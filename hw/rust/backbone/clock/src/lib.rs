//! Virtmcu deterministic clock with pluggable transport.
//!
//! This module provides the `VirtmcuClock` QOM device, which synchronizes
//! the guest's virtual time with an external TimeAuthority.

extern crate alloc;

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use core::ffi::{c_char, c_void, CStr};
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};
use core::time::Duration;
use crossbeam_channel::Receiver;
use std::sync::{Condvar, Mutex};
use std::time::Instant;
use virtmcu_api::{
    ClockAdvanceReq, ClockReadyResp, ClockSyncResponder, ClockSyncTransport, FlatBufferStructExt,
    CLOCK_ERROR_OK, CLOCK_ERROR_STALL,
};
use virtmcu_qom::cpu::CPUState;
use virtmcu_qom::qdev::SysBusDevice;
use virtmcu_qom::qom::{Object, ObjectClass, TypeInfo};
use virtmcu_qom::timer::{
    qemu_clock_get_ns, virtmcu_timer_free, virtmcu_timer_mod, virtmcu_timer_new_ns, QemuTimer,
    QEMU_CLOCK_VIRTUAL,
};
use virtmcu_qom::{
    declare_device_type, define_prop_bool, define_prop_string, define_prop_uint32,
    define_properties, device_class,
};
use zenoh::liveliness::LivelinessToken;
use zenoh::query::{Query, Queryable};
use zenoh::Session;
use zenoh::Wait;

/// Zenoh-based clock synchronization transport.
pub struct ZenohClockTransport {
    _queryable: Mutex<Option<Queryable<()>>>, // MUTEX_EXCEPTION: used for thread shutdown synchronization
    _liveliness: Option<LivelinessToken>,
    query_rx: Receiver<Query>,
    start_rx: Receiver<()>,
    _start_sub: Option<virtmcu_qom::sync::SafeSubscription>,
    _start_transport: Arc<transport_zenoh::ZenohDataTransport>,
    done_pub: alloc::sync::Arc<transport_zenoh::SafePublisher>,
    vtime_pub: alloc::sync::Arc<transport_zenoh::SafePublisher>,
    _node_id: u32,
    is_coordinated: bool,
}

impl ClockSyncTransport for ZenohClockTransport {
    fn recv_advance(
        &self,
        timeout: core::time::Duration,
    ) -> Option<(ClockAdvanceReq, Box<dyn ClockSyncResponder>)> {
        match self.query_rx.recv_timeout(timeout) {
            Ok(query) => {
                let data = query.payload().map(|p| p.to_bytes()).unwrap_or_default();
                ClockAdvanceReq::unpack_slice(&data).map(|req| {
                    let responder: Box<dyn ClockSyncResponder> = Box::new(ZenohClockResponder {
                        query,
                        start_rx: self.start_rx.clone(),
                        done_pub: alloc::sync::Arc::clone(&self.done_pub),
                        quantum: req.quantum_number(),
                        is_coordinated: self.is_coordinated,
                    });
                    (req, responder)
                })
            }
            Err(_) => None,
        }
    }

    fn send_vtime_heartbeat(&self, vtime_ns: u64) {
        let mut payload = alloc::vec::Vec::new();
        payload.extend_from_slice(&vtime_ns.to_le_bytes());
        self.vtime_pub.send(payload);
    }

    fn close(&self) {
        if let Ok(mut q) = self._queryable.lock() {
            if let Some(queryable) = q.take() {
                let _ = queryable.undeclare().wait();
            }
        }
    }
}

/// Zenoh-based clock synchronization responder.
pub struct ZenohClockResponder {
    query: Query,
    start_rx: Receiver<()>,
    done_pub: alloc::sync::Arc<transport_zenoh::SafePublisher>,
    quantum: u64,
    is_coordinated: bool,
}

impl ClockSyncResponder for ZenohClockResponder {
    fn send_ready(&self, resp: ClockReadyResp) -> Result<(), String> {
        if self.is_coordinated {
            // 1. Send 'done' signal to coordinator
            let mut payload = alloc::vec::Vec::new();
            payload.extend_from_slice(&self.quantum.to_le_bytes());
            self.done_pub.send(payload);

            // 2. Wait for 'start' signal from coordinator
            if let Err(e) = self.start_rx.recv() {
                virtmcu_qom::sim_err!(
                    "ARCH-8: start_rx channel disconnected before receiving start signal: {}",
                    e
                );
            }
        }

        // 3. Release the reply back to the Time Authority
        let resp_bytes = resp.pack();
        self.query
            .reply(self.query.key_expr().clone(), resp_bytes.to_vec())
            .wait()
            .map(|_| ())
            .map_err(|e| format!("Zenoh reply failed: {e}"))
    }
}

/* ── QOM Object ───────────────────────────────────────────────────────────── */

/// Deterministic clock device.
#[repr(C)]
pub struct VirtmcuClock {
    /// Parent object.
    pub parent_obj: SysBusDevice,

    /* Properties */
    /// Unique node ID for clock synchronization.
    pub node_id: u32,
    /// Synchronization mode ("slaved-suspend", "slaved-icount", "unix").
    pub mode: *mut c_char,
    /// Optional router address or socket path.
    pub router: *mut c_char,
    /// Timeout in milliseconds before a clock stall is declared.
    pub stall_timeout: u32,
    /// Whether to synchronize with a Deterministic Coordinator (ARCH-8).
    pub coordinated: bool,
    pub session_watchdog_ms: u32,

    /* Internal State */
    /// Virtual time (ns) of the next quantum boundary.
    pub next_quantum_ns: i64,
    /// Virtual time (ns) of the last halt event.
    pub last_halt_vtime: i64,
    /// Timer used to trigger quantum boundary checks.
    pub quantum_timer: *mut QemuTimer,

    /* Rust state */
    /// Opaque pointer to the Rust backend state.
    pub rust_state: *mut VirtmcuClockBackend,
    pub is_yielding: bool,
}

/// State of the quantum synchronization state machine.
#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum QuantumState {
    /// TA has granted a quantum, QEMU is executing instructions.
    Executing = 0,
    /// QEMU has reached a quantum boundary and is waiting for TA to grant the next one.
    Waiting = 1,
}

impl From<u8> for QuantumState {
    fn from(v: u8) -> Self {
        match v {
            0 => QuantumState::Executing,
            _ => QuantumState::Waiting,
        }
    }
}

/// Internal Rust backend for `VirtmcuClock`.
pub struct VirtmcuClockBackend {
    /// Optional Zenoh session for communication.
    pub session: Option<Arc<Session>>,
    /// Abstract transport for clock synchronization.
    pub transport: Box<dyn ClockSyncTransport>,
    /// Unique node ID.
    pub node_id: u32,
    /// Stall timeout in milliseconds.
    pub stall_timeout_ms: u32,
    /// Whether coordination is enabled for this node.
    pub is_coordinated: bool,
    pub watchdog_threshold: u64,
    pub consecutive_timeouts: core::sync::atomic::AtomicU64,
    pub abort_fn: alloc::sync::Arc<dyn Fn() + Send + Sync>,

    /* Communication state */
    /// Mutex for protecting communication state.
    pub mutex: Mutex<()>, // MUTEX_EXCEPTION: used with Condvar for cross-thread sync (vCPU <-> Worker)
    /// Condvar for signaling quantum events.
    pub cond: Condvar,

    /// Explicit state machine for quantum synchronization.
    pub state: core::sync::atomic::AtomicU8,
    /// Number of nanoseconds to advance in the current/next quantum.
    pub delta_ns: AtomicU64,
    /// Current virtual time in nanoseconds as known by the backend.
    pub vtime_ns: AtomicU64,
    /// Absolute simulation time in nanoseconds as reported by TimeAuthority.
    pub mujoco_time_ns: AtomicU64,
    /// Cumulative count of clock stalls.
    pub stall_count: AtomicU64,

    /* Profiling state */
    /// Total time spent waiting for the Big QEMU Lock (BQL).
    pub total_bql_wait_ns: AtomicU64,
    /// Total number of quantum iterations that acquired BQL.
    pub total_iterations: AtomicU64,
    /// Total number of quantum iterations that did NOT acquire BQL.
    pub total_no_bql_iterations: AtomicU64,
    /// Time when the backend was initialized.
    pub start_time: Instant,

    /// Whether this is the first quantum (allows longer timeout for boot).
    pub is_first_quantum: AtomicBool,

    /// Whether a stall was detected by the vCPU thread while waiting for a request.
    pub pending_stall: AtomicBool,

    /* Lifecycle */
    /// Whether the backend is shutting down.
    pub shutdown: Arc<AtomicBool>,
}

/* ── Logic ────────────────────────────────────────────────────────────────── */

static GLOBAL_CLOCK: AtomicPtr<VirtmcuClock> = AtomicPtr::new(ptr::null_mut());
static ACTIVE_HOOKS: AtomicU64 = AtomicU64::new(0);

extern "C" {
    fn virtmcu_kick_first_cpu_for_quantum();
    fn virtmcu_vcpu_should_yield() -> bool;
}

extern "C" fn clock_quantum_timer_cb(_opaque: *mut c_void) {
    // SAFETY: called from a QEMU timer callback; BQL is held by the QEMU main loop.
    unsafe { virtmcu_kick_first_cpu_for_quantum() };
}

extern "C" fn clock_cpu_tcg_hook(_cpu: *mut CPUState) {
    clock_cpu_halt_cb(_cpu, false);
}

struct ActiveHooksGuard;

impl ActiveHooksGuard {
    fn new() -> Self {
        ACTIVE_HOOKS.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

impl Drop for ActiveHooksGuard {
    fn drop(&mut self) {
        ACTIVE_HOOKS.fetch_sub(1, Ordering::SeqCst);
    }
}

virtmcu_api::virtmcu_export! {
    extern "C" fn clock_cpu_halt_cb(_cpu: *mut CPUState, halted: bool) {
        // 1. Signal that we are entering a hook
        let _guard = ActiveHooksGuard::new();

        // 2. Check if the clock device is still alive.
        let s_ptr = GLOBAL_CLOCK.load(Ordering::Acquire);
        if !s_ptr.is_null() {
            // SAFETY: s_ptr is checked for null and is a valid pointer to VirtmcuClock when not null.
            let s = unsafe { &mut *s_ptr };
            if !s.rust_state.is_null() {
                clock_cpu_halt_cb_internal(s, _cpu, halted);
            }
        }
    }
}

fn clock_cpu_halt_cb_internal(s: &mut VirtmcuClock, _cpu: *mut CPUState, halted: bool) {
    // Architectural change: if node_id is u32::MAX, we are in "bypass" mode.
    // This allows QEMU to boot and QMP to start before the test orchestrator
    // takes control and sets node_id via QMP.
    if s.node_id == u32::MAX {
        return;
    }

    // SAFETY: Calling qemu_clock_get_ns is safe under BQL or from vCPU thread.
    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };
    virtmcu_qom::telemetry::update_global_vtime(now as u64);

    if now >= s.next_quantum_ns {
        if s.rust_state.is_null() {
            return;
        }
        let backend = unsafe { &*s.rust_state };

        // Release BQL before blocking using RAII guard
        let bql_unlock = virtmcu_qom::sync::Bql::temporary_unlock();

        let raw_delta = clock_quantum_wait_internal(backend, now as u64, s.is_yielding);
        s.is_yielding = false;
        // On stall the sentinel is returned; treat as zero advance (hold position).
        let delta = if raw_delta == QUANTUM_WAIT_STALL_SENTINEL {
            0
        } else if raw_delta == QUANTUM_WAIT_YIELD_SENTINEL {
            s.is_yielding = true;
            0
        } else {
            raw_delta
        };

        if bql_unlock.is_some() {
            let bql_start = Instant::now();
            drop(bql_unlock); // Re-acquires BQL
            let bql_wait = bql_start.elapsed().as_nanos() as u64;
            backend.total_bql_wait_ns.fetch_add(bql_wait, Ordering::Relaxed);
            backend.total_iterations.fetch_add(1, Ordering::Relaxed);
        } else {
            backend.total_no_bql_iterations.fetch_add(1, Ordering::Relaxed);
        }

        // Advance virtual clock manually if requested by TA.
        let target_vtime = now + delta as i64;
        // SAFETY: Calling qemu_clock_get_ns is safe under BQL or from vCPU thread.
        let now_after_block = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };

        if delta > 0 {
            let should_advance = !virtmcu_qom::icount::icount_enabled() || halted;
            if should_advance && target_vtime > now_after_block {
                virtmcu_qom::icount::icount_advance(target_vtime - now_after_block);
            }
        }

        // Set next boundary
        s.next_quantum_ns = target_vtime;

        // Final safety: ensure it's always in the future relative to final time.
        // SAFETY: Calling qemu_clock_get_ns is safe under BQL or from vCPU thread.
        let now_final = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };
        if s.next_quantum_ns < now_final {
            s.next_quantum_ns = now_final;
        }

        if !s.quantum_timer.is_null() {
            // SAFETY: s.quantum_timer is checked for null and is a valid QEMU timer.
            unsafe {
                virtmcu_timer_mod(s.quantum_timer, s.next_quantum_ns);
            }
        }
    }
}

/// Return value of `clock_quantum_wait_internal`: delta_ns on success,
/// or `u64::MAX` as a sentinel indicating a stall timeout.
const QUANTUM_WAIT_STALL_SENTINEL: u64 = u64::MAX;
const QUANTUM_WAIT_YIELD_SENTINEL: u64 = u64::MAX - 1;

fn clock_quantum_wait_internal(
    backend: &VirtmcuClockBackend,
    _vtime_ns: u64,
    is_yielding: bool,
) -> u64 {
    // Runtime assertion (not just debug_assert): BQL must NOT be held here.
    if virtmcu_qom::sync::Bql::is_held() {
        if virtmcu_qom::sysemu::runstate_is_running() {
            virtmcu_qom::sim_warn!(
                "BQL held entering quantum_wait — would deadlock. Skipping sync."
            );
        }
        return QUANTUM_WAIT_STALL_SENTINEL;
    }

    backend.vtime_ns.store(_vtime_ns, Ordering::SeqCst);

    if !is_yielding {
        // Transition: Initial -> Waiting
        let current_state = QuantumState::from(backend.state.load(Ordering::Acquire));
        if current_state != QuantumState::Waiting {
            let _ = backend.state.compare_exchange(
                current_state as u8,
                QuantumState::Waiting as u8,
                Ordering::SeqCst,
                Ordering::Relaxed,
            );
        }

        // Notify TA that we finished previous quantum
        {
            let _guard = backend.mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            backend.cond.notify_all();
        }
    }

    let start = Instant::now();
    let is_first = backend.is_first_quantum.load(Ordering::Relaxed);
    let timeout = if is_first {
        BOOT_QUANTUM_TIMEOUT
    } else {
        Duration::from_millis(u64::from(backend.stall_timeout_ms))
    };

    // Spin briefly to avoid context switch latency
    while backend.state.load(Ordering::SeqCst) != QuantumState::Executing as u8 {
        if backend.shutdown.load(Ordering::Acquire) {
            return 0;
        }
        if unsafe { virtmcu_vcpu_should_yield() } {
            return QUANTUM_WAIT_YIELD_SENTINEL;
        }
        if start.elapsed() > Duration::from_millis(1) {
            break;
        }
        core::hint::spin_loop();
    }

    if backend.state.load(Ordering::SeqCst) != QuantumState::Executing as u8 {
        let mut guard = backend.mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        while backend.state.load(Ordering::SeqCst) != QuantumState::Executing as u8 {
            if backend.shutdown.load(Ordering::Acquire) {
                return 0;
            }
            if unsafe { virtmcu_vcpu_should_yield() } {
                return QUANTUM_WAIT_YIELD_SENTINEL;
            }

            // Wait for Executing
            let (new_guard, result) = backend
                .cond
                .wait_timeout(guard, Duration::from_millis(100))
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard = new_guard;

            if result.timed_out() && start.elapsed() > timeout {
                backend.stall_count.fetch_add(1, Ordering::Relaxed);
                backend.pending_stall.store(true, Ordering::SeqCst);
                return QUANTUM_WAIT_STALL_SENTINEL;
            }
        }
    }

    backend.delta_ns.load(Ordering::SeqCst)
}

const BOOT_QUANTUM_TIMEOUT: Duration = Duration::from_mins(10);

fn clock_worker_loop(backend: Arc<VirtmcuClockBackend>) {
    virtmcu_qom::sim_info!("Worker thread for node {} started.", backend.node_id);

    let mut last_report = Instant::now();
    while !backend.shutdown.load(Ordering::Acquire) {
        let is_first = backend.is_first_quantum.load(Ordering::Relaxed);
        let timeout = if is_first {
            BOOT_QUANTUM_TIMEOUT
        } else {
            Duration::from_millis(backend.stall_timeout_ms as u64)
        };

        let (req, responder) = match backend.transport.recv_advance(timeout) {
            Some(r) => {
                backend.consecutive_timeouts.store(0, Ordering::Relaxed);
                r
            }
            None => {
                if backend.shutdown.load(Ordering::Acquire) {
                    break;
                }

                if !is_first {
                    let misses = backend.consecutive_timeouts.fetch_add(1, Ordering::Relaxed) + 1;
                    if misses > backend.watchdog_threshold {
                        (backend.abort_fn)();
                        return;
                    }
                }
                continue;
            }
        };

        let delta = req.delta_ns();
        let mujoco = req.mujoco_time_ns();

        backend.delta_ns.store(delta, Ordering::SeqCst);
        backend.mujoco_time_ns.store(mujoco, Ordering::SeqCst);

        let mut error_code = wait_for_ready_and_execute(&backend, delta, timeout, is_first);

        if backend.pending_stall.swap(false, Ordering::SeqCst) {
            error_code = CLOCK_ERROR_STALL;
        }

        let current_vtime = backend.vtime_ns.load(Ordering::SeqCst);
        backend.transport.send_vtime_heartbeat(current_vtime);

        let resp = ClockReadyResp::new(current_vtime, 0, error_code, req.quantum_number());

        if let Err(e) = responder.send_ready(resp) {
            virtmcu_qom::sim_err!("{}", e);
        }

        if last_report.elapsed() >= Duration::from_secs(1) {
            report_contention(&backend, &mut last_report);
        }
    }
}

fn wait_for_ready_and_execute(
    backend: &Arc<VirtmcuClockBackend>,
    delta: u64,
    timeout: Duration,
    is_first: bool,
) -> u32 {
    let start = Instant::now();

    loop {
        if backend.shutdown.load(Ordering::Acquire) {
            return CLOCK_ERROR_OK;
        }
        let current_state = backend.state.load(Ordering::SeqCst);
        if current_state == QuantumState::Waiting as u8 {
            break;
        }

        if start.elapsed() > timeout {
            return CLOCK_ERROR_STALL;
        }

        let guard = backend.mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut _new_guard, _) = backend
            .cond
            .wait_timeout(guard, Duration::from_millis(10))
            .unwrap_or_else(std::sync::PoisonError::into_inner);
    }

    if backend
        .state
        .compare_exchange(
            QuantumState::Waiting as u8,
            QuantumState::Executing as u8,
            Ordering::SeqCst,
            Ordering::Relaxed,
        )
        .is_err()
    {
        virtmcu_qom::sim_err!("Invalid state transition");
    }

    {
        let _guard = backend.mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        backend.cond.notify_all();
    }

    if delta > 0 {
        let exec_start = Instant::now();
        loop {
            if backend.shutdown.load(Ordering::Acquire) {
                return CLOCK_ERROR_OK;
            }
            let current_state = backend.state.load(Ordering::SeqCst);
            if current_state == QuantumState::Waiting as u8 {
                break;
            }

            if exec_start.elapsed() > timeout {
                return CLOCK_ERROR_STALL;
            }

            let guard = backend.mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let (mut _new_guard, _) = backend
                .cond
                .wait_timeout(guard, Duration::from_millis(10))
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    if is_first {
        backend.is_first_quantum.store(false, Ordering::Relaxed);
    }

    CLOCK_ERROR_OK
}

fn report_contention(backend: &VirtmcuClockBackend, last_report: &mut Instant) {
    let iterations = backend.total_iterations.swap(0, Ordering::Relaxed);
    let no_bql = backend.total_no_bql_iterations.swap(0, Ordering::Relaxed);
    let total_wait = backend.total_bql_wait_ns.swap(0, Ordering::Relaxed);
    let elapsed = last_report.elapsed().as_secs_f64();

    if iterations > 0 || no_bql > 0 {
        let contention = (total_wait as f64 / (elapsed * 1_000_000_000.0)) * 100.0;
        virtmcu_qom::sim_info!("{:.2}% (samples: {}, no_bql: {})", contention, iterations, no_bql);
    }
    *last_report = Instant::now();
}

unsafe extern "C" fn clock_realize(dev: *mut c_void, errp: *mut *mut c_void) {
    let s = &mut *(dev as *mut VirtmcuClock);
    virtmcu_qom::telemetry::update_global_node_id(s.node_id);

    let mode_str = if s.mode.is_null() {
        "slaved-suspend"
    } else {
        unsafe { CStr::from_ptr(s.mode) }.to_str().unwrap_or("slaved-suspend")
    };

    let router_str = if s.router.is_null() { ptr::null() } else { s.router.cast_const() };

    let is_unix = mode_str == "unix" || mode_str == "slaved-unix";

    let mut stall_ms = s.stall_timeout;
    if stall_ms == 0 {
        stall_ms = 5000;
    }

    let is_coordinated = s.coordinated;

    if is_unix {
        if router_str.is_null() {
            virtmcu_qom::error_setg!(errp, "clock: 'router' (socket path) required for unix\n");
            return;
        }
        let path = unsafe { CStr::from_ptr(router_str) }.to_string_lossy();
        let transport = virtmcu_api::UnixSocketClockTransport::new(path.as_ref());
        let watchdog_ms =
            if s.session_watchdog_ms > 0 { s.session_watchdog_ms } else { stall_ms * 3 };
        s.rust_state = clock_init_with_transport(
            s.node_id,
            Box::new(transport),
            None,
            stall_ms,
            is_coordinated,
            watchdog_ms,
        );
    } else {
        let watchdog_ms =
            if s.session_watchdog_ms > 0 { s.session_watchdog_ms } else { stall_ms * 3 };
        s.rust_state =
            clock_init_internal(s.node_id, router_str, stall_ms, is_coordinated, watchdog_ms);
    }

    if s.rust_state.is_null() {
        virtmcu_qom::error_setg!(errp, "clock: failed to initialize Rust backend");
        return;
    }

    s.next_quantum_ns = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };
    s.last_halt_vtime = -1;
    s.quantum_timer =
        unsafe { virtmcu_timer_new_ns(QEMU_CLOCK_VIRTUAL, clock_quantum_timer_cb, dev) };

    unsafe {
        virtmcu_timer_mod(s.quantum_timer, s.next_quantum_ns);
    }

    let prev = GLOBAL_CLOCK.swap(s, Ordering::AcqRel);
    if !prev.is_null() {
        std::process::abort();
    }

    unsafe {
        virtmcu_qom::cpu::virtmcu_cpu_set_halt_hook(Some(clock_cpu_halt_cb));
        virtmcu_qom::cpu::virtmcu_cpu_set_tcg_hook(Some(clock_cpu_tcg_hook));
    }
}

unsafe extern "C" fn clock_instance_finalize(obj: *mut Object) {
    let s = &mut *(obj as *mut VirtmcuClock);
    GLOBAL_CLOCK.store(ptr::null_mut(), Ordering::Release);
    unsafe {
        virtmcu_qom::cpu::virtmcu_cpu_set_halt_hook(None);
        virtmcu_qom::cpu::virtmcu_cpu_set_tcg_hook(None);
    }

    if !s.rust_state.is_null() {
        let backend = unsafe { Arc::from_raw(s.rust_state) };
        backend.shutdown.store(true, Ordering::Release);
        backend.transport.close();

        let mut guard = backend.mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        backend.cond.notify_all();
        while ACTIVE_HOOKS.load(Ordering::SeqCst) > 0 {
            let bql_unlock = virtmcu_qom::sync::Bql::temporary_unlock();
            let (new_guard, _) = backend
                .cond
                .wait_timeout(guard, core::time::Duration::from_millis(100))
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard = new_guard;
            drop(bql_unlock);
        }
        s.rust_state = ptr::null_mut();
    }
    if !s.quantum_timer.is_null() {
        unsafe {
            virtmcu_timer_free(s.quantum_timer);
        }
        s.quantum_timer = ptr::null_mut();
    }
}

unsafe extern "C" fn clock_instance_init(obj: *mut Object) {
    let s = &mut *(obj as *mut VirtmcuClock);
    s.rust_state = ptr::null_mut();
    s.quantum_timer = ptr::null_mut();
    s.node_id = u32::MAX;
}

define_properties!(
    VIRT_CLOCK_PROPERTIES,
    [
        define_prop_uint32!(c"node".as_ptr(), VirtmcuClock, node_id, 0xFFFF_FFFF),
        define_prop_string!(c"mode".as_ptr(), VirtmcuClock, mode),
        define_prop_string!(c"router".as_ptr(), VirtmcuClock, router),
        define_prop_uint32!(c"stall-timeout".as_ptr(), VirtmcuClock, stall_timeout, 0),
        define_prop_bool!(c"coordinated".as_ptr(), VirtmcuClock, coordinated, false),
        define_prop_uint32!(c"session-watchdog-ms".as_ptr(), VirtmcuClock, session_watchdog_ms, 0),
    ]
);

unsafe extern "C" fn clock_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let dc = device_class!(klass);
    unsafe {
        (*dc).realize = Some(clock_realize);
        (*dc).user_creatable = true;
    }
    virtmcu_qom::device_class_set_props!(dc, VIRT_CLOCK_PROPERTIES);
}

static VIRT_CLOCK_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"virtmcu-clock".as_ptr(),
    parent: c"sys-bus-device".as_ptr(),
    instance_size: core::mem::size_of::<VirtmcuClock>(),
    instance_align: 0,
    instance_init: Some(clock_instance_init),
    instance_post_init: None,
    instance_finalize: Some(clock_instance_finalize),
    abstract_: false,
    class_size: 0,
    class_init: Some(clock_class_init),
    class_base_init: None,
    class_data: ptr::null(),
    interfaces: ptr::null(),
};

declare_device_type!(VIRT_CLOCK_TYPE_INIT, VIRT_CLOCK_TYPE_INFO);

fn clock_init_with_transport(
    node_id: u32,
    transport: Box<dyn ClockSyncTransport>,
    session: Option<Arc<Session>>,
    stall_timeout_ms: u32,
    is_coordinated: bool,
    session_watchdog_ms: u32,
) -> *mut VirtmcuClockBackend {
    let shutdown = Arc::new(AtomicBool::new(false));
    let watchdog_threshold = if session_watchdog_ms > 0 && stall_timeout_ms > 0 {
        (session_watchdog_ms / stall_timeout_ms) as u64
    } else {
        3
    };
    let backend = Arc::new(VirtmcuClockBackend {
        session,
        transport,
        node_id,
        stall_timeout_ms,
        is_coordinated,
        watchdog_threshold,
        consecutive_timeouts: core::sync::atomic::AtomicU64::new(0),
        abort_fn: Arc::new(|| std::process::exit(1)),
        mutex: Mutex::new(()),
        cond: Condvar::new(),
        state: core::sync::atomic::AtomicU8::new(QuantumState::Waiting as u8),
        delta_ns: AtomicU64::new(0),
        vtime_ns: AtomicU64::new(0),
        mujoco_time_ns: AtomicU64::new(0),
        stall_count: AtomicU64::new(0),
        total_bql_wait_ns: AtomicU64::new(0),
        total_iterations: AtomicU64::new(0),
        total_no_bql_iterations: AtomicU64::new(0),
        start_time: Instant::now(),
        is_first_quantum: AtomicBool::new(true),
        pending_stall: AtomicBool::new(false),
        shutdown: Arc::clone(&shutdown),
    });
    let backend_ptr = Arc::into_raw(backend);
    let worker_backend = unsafe {
        let b = Arc::from_raw(backend_ptr);
        let clone = Arc::clone(&b);
        let _ = Arc::into_raw(b);
        clone
    };
    std::thread::spawn(move || clock_worker_loop(worker_backend));
    backend_ptr.cast_mut()
}

fn clock_init_internal(
    node_id: u32,
    router: *const c_char,
    stall_timeout_ms: u32,
    is_coordinated: bool,
    session_watchdog_ms: u32,
) -> *mut VirtmcuClockBackend {
    let session = unsafe {
        match transport_zenoh::open_session(router) {
            Ok(s) => Arc::new(s),
            Err(_) => return ptr::null_mut(),
        }
    };
    let (query_tx, query_rx) = crossbeam_channel::unbounded();
    let (start_tx, start_rx) = crossbeam_channel::unbounded();
    let start_topic = format!("sim/clock/start/{node_id}");
    let start_transport = Arc::new(transport_zenoh::ZenohDataTransport::new(Arc::clone(&session)));
    let _start_sub = virtmcu_qom::sync::SafeSubscription::new(
        start_transport.as_ref(),
        &start_topic,
        Arc::new(AtomicU64::new(0)),
        Box::new(move |_| {
            let _ = start_tx.send(());
        }),
    )
    .ok();

    let done_topic = format!("sim/coord/{node_id}/done");
    let publisher = match session.declare_publisher(done_topic).wait() {
        Ok(p) => p,
        Err(_) => return ptr::null_mut(),
    };
    let done_pub = Arc::new(transport_zenoh::SafePublisher::new(publisher));

    let vtime_topic = format!("sim/clock/vtime/{node_id}");
    let vtime_publisher = match session.declare_publisher(vtime_topic).wait() {
        Ok(p) => p,
        Err(_) => return ptr::null_mut(),
    };
    let vtime_pub = Arc::new(transport_zenoh::SafePublisher::new(vtime_publisher));

    let topic = format!("sim/clock/advance/{node_id}");
    let queryable = match session
        .declare_queryable(topic.clone())
        .callback(move |query| {
            let _ = query_tx.send(query);
        })
        .wait()
    {
        Ok(q) => q,
        Err(_) => return ptr::null_mut(),
    };
    let hb_topic = format!("sim/clock/liveliness/{node_id}");
    let _liveliness = session.liveliness().declare_token(hb_topic).wait().ok();
    let transport = Box::new(ZenohClockTransport {
        _queryable: Mutex::new(Some(queryable)),
        _liveliness,
        query_rx,
        start_rx,
        _start_sub,
        _start_transport: start_transport,
        done_pub,
        vtime_pub,
        _node_id: node_id,
        is_coordinated,
    });
    clock_init_with_transport(
        node_id,
        transport,
        Some(session),
        stall_timeout_ms,
        is_coordinated,
        session_watchdog_ms,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_clock_layout() {
        assert_eq!(core::mem::offset_of!(VirtmcuClock, parent_obj), 0);
    }
}
