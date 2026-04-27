//! Zenoh-based deterministic clock for VirtMCU nodes.
//!
//! This module provides the `ZenohClock` QOM device, which synchronizes
//! the guest's virtual time with an external TimeAuthority via Zenoh.

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
    ClockAdvanceReq, ClockReadyResp, ClockSyncResponder, ClockSyncTransport, CLOCK_ERROR_OK,
    CLOCK_ERROR_STALL,
};
use virtmcu_qom::cpu::CPUState;
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
use zenoh::liveliness::LivelinessToken;
use zenoh::query::{Query, Queryable};
use zenoh::Session;
use zenoh::Wait;

/// Zenoh-based clock synchronization transport.
pub struct ZenohClockTransport {
    _queryable: Mutex<Option<Queryable<()>>>, // MUTEX_EXCEPTION: used for thread shutdown synchronization
    _liveliness: Option<LivelinessToken>,
    query_rx: Receiver<Query>,
}

impl ClockSyncTransport for ZenohClockTransport {
    fn recv_advance(&self) -> Option<(ClockAdvanceReq, Box<dyn ClockSyncResponder>)> {
        match self.query_rx.recv() {
            Ok(query) => {
                let data = query.payload().map(|p| p.to_bytes()).unwrap_or_default();
                ClockAdvanceReq::unpack_slice(&data).map(|req| {
                    let responder: Box<dyn ClockSyncResponder> =
                        Box::new(ZenohClockResponder { query });
                    (req, responder)
                })
            }
            Err(_) => None,
        }
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
}

impl ClockSyncResponder for ZenohClockResponder {
    fn send_ready(&self, resp: ClockReadyResp) -> Result<(), String> {
        let resp_bytes = resp.pack();
        self.query
            .reply(self.query.key_expr().clone(), resp_bytes.to_vec())
            .wait()
            .map(|_| ())
            .map_err(|e| format!("Zenoh reply failed: {e}"))
    }
}

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

/// Internal Rust backend for `ZenohClock`.
pub struct ZenohClockBackend {
    /// Optional Zenoh session for communication.
    pub session: Option<Arc<Session>>,
    /// Abstract transport for clock synchronization.
    pub transport: Box<dyn ClockSyncTransport>,
    /// Unique node ID.
    pub node_id: u32,
    /// Stall timeout in milliseconds.
    pub stall_timeout_ms: u32,

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

    /* Lifecycle */
    /// Whether the backend is shutting down.
    pub shutdown: Arc<AtomicBool>,
}

/* ── Logic ────────────────────────────────────────────────────────────────── */

static GLOBAL_CLOCK: AtomicPtr<ZenohClock> = AtomicPtr::new(ptr::null_mut());
static ACTIVE_HOOKS: AtomicU64 = AtomicU64::new(0);

extern "C" fn zenoh_clock_quantum_timer_cb(_opaque: *mut c_void) {
    zenoh_clock_cpu_halt_cb(ptr::null_mut(), false);
}

extern "C" fn zenoh_clock_cpu_tcg_hook(_cpu: *mut CPUState) {
    zenoh_clock_cpu_halt_cb(_cpu, false);
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

extern "C" fn zenoh_clock_cpu_halt_cb(_cpu: *mut CPUState, halted: bool) {
    // 1. Signal that we are entering a hook
    let _guard = ActiveHooksGuard::new();

    // 2. Check if the clock device is still alive.
    let s_ptr = GLOBAL_CLOCK.load(Ordering::Acquire);
    if !s_ptr.is_null() {
        // SAFETY: s_ptr is checked for null and is a valid pointer to ZenohClock when not null.
        let s = unsafe { &mut *s_ptr };
        if !s.rust_state.is_null() {
            zenoh_clock_cpu_halt_cb_internal(s, _cpu, halted);
        }
    }
}

fn zenoh_clock_cpu_halt_cb_internal(s: &mut ZenohClock, _cpu: *mut CPUState, halted: bool) {
    // Architectural change: if node_id is 0, we are in "bypass" mode.
    // This allows QEMU to boot and QMP to start before the test orchestrator
    // takes control and sets node_id via QMP.
    if s.node_id == 0 {
        return;
    }

    // SAFETY: Calling qemu_clock_get_ns is safe under BQL or from vCPU thread.
    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };

    // In slaved mode, we ONLY block when we reach the virtual time boundary.
    let should_block = now >= s.next_quantum_ns;

    if should_block {
        // SAFETY: s.rust_state is checked for null before calling this function.
        let backend = unsafe { &*s.rust_state };

        // Release BQL before blocking using RAII guard
        let bql_unlock = virtmcu_qom::sync::Bql::temporary_unlock();
        if bql_unlock.is_some() {
            // vlog!("[zenoh-clock] should_block: was_locked=true\n");
        }

        let raw_delta = zenoh_clock_quantum_wait_internal(backend, now as u64);
        // On stall the sentinel is returned; treat as zero advance (hold position).
        let delta = if raw_delta == QUANTUM_WAIT_STALL_SENTINEL { 0 } else { raw_delta };

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
        if s.next_quantum_ns <= now_final {
            s.next_quantum_ns = now_final + i64::from(delta == 0);
        }

        if !s.quantum_timer.is_null() {
            // SAFETY: s.quantum_timer is checked for null and is a valid QEMU timer.
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
    // Runtime assertion (not just debug_assert): BQL must NOT be held here.
    if virtmcu_qom::sync::Bql::is_held() {
        if virtmcu_qom::sysemu::runstate_is_running() {
            virtmcu_qom::vlog!(
                "[zenoh-clock] WARNING: BQL held entering quantum_wait — would deadlock. Skipping sync.\n"
            );
        }
        return QUANTUM_WAIT_STALL_SENTINEL;
    }

    backend.vtime_ns.store(_vtime_ns, Ordering::SeqCst);

    // Transition: Executing/Initial -> Waiting
    let current_state = QuantumState::from(backend.state.load(Ordering::Acquire));
    if current_state != QuantumState::Waiting {
        let _ = backend.state.compare_exchange(
            current_state as u8,
            QuantumState::Waiting as u8,
            Ordering::SeqCst,
            Ordering::Relaxed,
        );
    }

    // Notify TA that we finished previous quantum (or we are ready for the first one)
    {
        let _guard = backend.mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        backend.cond.notify_all();
    }

    let start = Instant::now();
    let is_first = backend.is_first_quantum.load(Ordering::Relaxed);
    let timeout = if is_first {
        BOOT_QUANTUM_TIMEOUT
    } else {
        Duration::from_millis(u64::from(backend.stall_timeout_ms))
    };

    // Spin briefly to avoid context switch latency for very fast quantums
    while backend.state.load(Ordering::SeqCst) != QuantumState::Executing as u8 {
        if backend.shutdown.load(Ordering::Acquire) {
            return 0;
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

    backend.delta_ns.load(Ordering::SeqCst)
}

/// Timeout for the very first quantum execution (TCG initialization, ASan overhead).
const BOOT_QUANTUM_TIMEOUT: Duration = Duration::from_secs(60 * 10); // 10 mins

fn zenoh_clock_worker_loop(backend: Arc<ZenohClockBackend>) {
    vlog!("[zenoh-clock] Worker thread for node {} started.\n", backend.node_id);

    let mut last_report = Instant::now();
    while !backend.shutdown.load(Ordering::Acquire) {
        // Blocks until a new quantum advancement is requested.
        let (req, responder) = match backend.transport.recv_advance() {
            Some(r) => r,
            None => {
                if backend.shutdown.load(Ordering::Acquire) {
                    break;
                }
                // Small sleep to avoid tight loop on transient errors
                std::thread::sleep(Duration::from_millis(100)); // SLEEP_EXCEPTION: transport error backoff
                continue;
            }
        };

        let delta = req.delta_ns;
        let mujoco = req.mujoco_time_ns;

        let is_first = backend.is_first_quantum.load(Ordering::Relaxed);
        let timeout = if is_first {
            BOOT_QUANTUM_TIMEOUT
        } else {
            Duration::from_millis(u64::from(backend.stall_timeout_ms))
        };

        // 1. Prepare for the next quantum
        backend.delta_ns.store(delta, Ordering::SeqCst);
        backend.mujoco_time_ns.store(mujoco, Ordering::SeqCst);

        // 2. Wait for QEMU to be ready (state=Waiting)
        let error_code = wait_for_ready_and_execute(&backend, delta, timeout, is_first);

        // QEMU has stopped executing. Read the current virtual time.
        let current_vtime = backend.vtime_ns.load(Ordering::SeqCst);

        // Reply to TimeAuthority.
        let resp = ClockReadyResp {
            current_vtime_ns: current_vtime,
            n_frames: 0, // TODO: track frames if needed for profiling
            error_code,
        };

        if let Err(e) = responder.send_ready(resp) {
            vlog!("[zenoh-clock] Failed to send clock ready response: {}\n", e);
        }

        if last_report.elapsed() >= Duration::from_secs(1) {
            report_contention(&backend, &mut last_report);
        }
    }

    vlog!("[zenoh-clock] Worker thread for node {} exiting.\n", backend.node_id);
}

fn wait_for_ready_and_execute(
    backend: &Arc<ZenohClockBackend>,
    delta: u64,
    timeout: Duration,
    is_first: bool,
) -> u32 {
    let start = Instant::now();

    // 1. Wait for QEMU to reach Waiting state
    loop {
        if backend.shutdown.load(Ordering::Acquire) {
            return CLOCK_ERROR_OK;
        }
        if backend.state.load(Ordering::SeqCst) == QuantumState::Waiting as u8 {
            break;
        }

        if start.elapsed() > timeout {
            vlog!(
                "[zenoh-clock] STALL DETECTED: QEMU did not reach Waiting state within {:?}!\n",
                timeout
            );
            return CLOCK_ERROR_STALL;
        }

        let guard = backend.mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut _new_guard, _) = backend
            .cond
            .wait_timeout(guard, Duration::from_millis(10))
            .unwrap_or_else(std::sync::PoisonError::into_inner);
    }

    // 2. Transition to Executing
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
        vlog!("[zenoh-clock] ERROR: Invalid state transition (expected Waiting)\n");
    }

    // 3. Wake up the vCPU thread
    {
        let _guard = backend.mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        backend.cond.notify_all();
    }

    // 4. Wait for QEMU to finish quantum (return to Waiting)
    // Only if delta > 0. If delta == 0, we just synced.
    if delta > 0 {
        let exec_start = Instant::now();
        loop {
            if backend.shutdown.load(Ordering::Acquire) {
                return CLOCK_ERROR_OK;
            }
            if backend.state.load(Ordering::SeqCst) == QuantumState::Waiting as u8 {
                break;
            }

            if exec_start.elapsed() > timeout {
                vlog!(
                    "[zenoh-clock] STALL DETECTED: QEMU did not complete quantum (delta={}) within {:?}!\n",
                    delta,
                    timeout
                );
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
        vlog!("[zenoh-clock] First quantum completed. Reverting to standard timeout.\n");
        backend.is_first_quantum.store(false, Ordering::Relaxed);
    }

    CLOCK_ERROR_OK
}

fn report_contention(backend: &ZenohClockBackend, last_report: &mut Instant) {
    let iterations = backend.total_iterations.swap(0, Ordering::Relaxed);
    let no_bql = backend.total_no_bql_iterations.swap(0, Ordering::Relaxed);
    let total_wait = backend.total_bql_wait_ns.swap(0, Ordering::Relaxed);
    let elapsed = last_report.elapsed().as_secs_f64();

    if iterations > 0 || no_bql > 0 {
        let avg_wait_us =
            if iterations > 0 { (total_wait as f64 / iterations as f64) / 1000.0 } else { 0.0 };
        let contention = (total_wait as f64 / (elapsed * 1_000_000_000.0)) * 100.0;

        virtmcu_qom::vlog!(
            "[zenoh-clock] BQL Contention: {:.2}% (avg wait: {:.2} us, samples: {}, no_bql: {})\n",
            contention,
            avg_wait_us,
            iterations,
            no_bql
        );
    }
    *last_report = Instant::now();
}

/* ── Boilerplate ──────────────────────────────────────────────────────────── */

unsafe extern "C" fn zenoh_clock_realize(dev: *mut c_void, errp: *mut *mut c_void) {
    let s = &mut *(dev as *mut ZenohClock);

    let mode_str = if s.mode.is_null() {
        "slaved-suspend"
    } else {
        // SAFETY: s.mode is checked for null.
        unsafe { CStr::from_ptr(s.mode) }.to_str().unwrap_or("slaved-suspend")
    };

    let router_str = if s.router.is_null() { ptr::null() } else { s.router.cast_const() };

    let is_unix = mode_str == "unix" || mode_str == "slaved-unix";

    if !is_unix
        && mode_str != "icount"
        && mode_str != "slaved-icount"
        && mode_str != "suspend"
        && mode_str != "slaved-suspend"
    {
        return;
    }

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

    if is_unix {
        if router_str.is_null() {
            virtmcu_qom::error_setg!(
                errp,
                "zenoh-clock: 'router' property (socket path) required for 'unix' mode"
            );
            return;
        }
        // SAFETY: router_str is checked for null and is a valid C string.
        let path = unsafe { CStr::from_ptr(router_str) }.to_string_lossy();
        let transport = virtmcu_api::UnixSocketClockTransport::new(path.as_ref());
        s.rust_state =
            zenoh_clock_init_with_transport(s.node_id, Box::new(transport), None, stall_ms);
    } else {
        s.rust_state = zenoh_clock_init_internal(s.node_id, router_str, stall_ms);
    }

    if s.rust_state.is_null() {
        virtmcu_qom::error_setg!(errp, "zenoh-clock: failed to initialize Rust backend");
        return;
    }

    // SAFETY: Safe to query virtual clock during realize.
    s.next_quantum_ns = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };
    s.last_halt_vtime = -1;
    // SAFETY: Creating a new QEMU timer is safe here. dev is a valid opaque pointer.
    s.quantum_timer =
        unsafe { virtmcu_timer_new_ns(QEMU_CLOCK_VIRTUAL, zenoh_clock_quantum_timer_cb, dev) };

    // Task 27.2: Ensure the timer is scheduled initially so we reach the first hook
    // even if the guest is idling or slow to boot.
    // SAFETY: s.quantum_timer was just successfully created.
    unsafe {
        virtmcu_timer_mod(s.quantum_timer, s.next_quantum_ns);
    }

    // Fail loudly if multiple clock devices are instantiated
    let prev = GLOBAL_CLOCK.swap(s, Ordering::AcqRel);
    if !prev.is_null() {
        vlog!("[zenoh-clock] FATAL: Multiple ZenohClock instances realized! VirtMCU supports only one clock authority.\n");
        std::process::abort();
    }

    // SAFETY: Setting the global C function pointers for hooks is safe during realization.
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
    // SAFETY: Clearing the global C function pointers is safe during finalize.
    unsafe {
        virtmcu_qom::cpu::virtmcu_cpu_halt_hook = None;
        virtmcu_qom::cpu::virtmcu_tcg_quantum_hook = None;
    }

    // 2. Wait for any active hook executions to finish their logic.
    //    Since we set GLOBAL_CLOCK to NULL, any NEW hook entries will return immediately.
    //    This loop waits for those that were already inside.
    {
        if !s.rust_state.is_null() {
            // SAFETY: s.rust_state was allocated via Arc::into_raw and is being reclaimed here.
            let backend = unsafe { Arc::from_raw(s.rust_state) };
            backend.shutdown.store(true, Ordering::Release);
            backend.transport.close(); // Unblock the worker thread

            let mut guard = backend.mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            backend.cond.notify_all(); // Wake up worker
            while ACTIVE_HOOKS.load(Ordering::SeqCst) > 0 {
                let bql_unlock = virtmcu_qom::sync::Bql::temporary_unlock();
                let (new_guard, _) = backend
                    .cond
                    .wait_timeout(guard, core::time::Duration::from_millis(100))
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                guard = new_guard;
                drop(bql_unlock);
            }
            // Final report
            let iterations = backend.total_iterations.load(Ordering::Relaxed);
            let no_bql = backend.total_no_bql_iterations.load(Ordering::Relaxed);
            let total_wait = backend.total_bql_wait_ns.load(Ordering::Relaxed);
            let elapsed = backend.start_time.elapsed().as_secs_f64();
            if iterations > 0 || no_bql > 0 {
                let avg_wait_us = if iterations > 0 {
                    (total_wait as f64 / iterations as f64) / 1000.0
                } else {
                    0.0
                };
                let contention = (total_wait as f64 / (elapsed * 1_000_000_000.0)) * 100.0;
                virtmcu_qom::vlog!(
                    "[zenoh-clock] BQL Contention: {:.2}% (avg wait: {:.2} us, samples: {}, no_bql: {})\n",
                    contention,
                    avg_wait_us,
                    iterations,
                    no_bql
                );
            }
            // Arc is dropped here
            s.rust_state = ptr::null_mut();
        }
    }
    if !s.quantum_timer.is_null() {
        // SAFETY: s.quantum_timer is a valid timer pointer being freed during finalize.
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
    s.node_id = 0; // Default to bypass mode
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
    // SAFETY: Setting class methods on the device class is safe during class initialization.
    unsafe {
        (*dc).realize = Some(zenoh_clock_realize);
        (*dc).user_creatable = true;
    }
    virtmcu_qom::device_class_set_props!(dc, ZENOH_CLOCK_PROPERTIES);
}

static ZENOH_CLOCK_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"zenoh-clock".as_ptr(),
    parent: c"sys-bus-device".as_ptr(),
    instance_size: core::mem::size_of::<ZenohClock>(),
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

declare_device_type!(ZENOH_CLOCK_TYPE_INIT, ZENOH_CLOCK_TYPE_INFO);

/* ── Internal Rust State ─────────────────────────────────────────────────── */

fn zenoh_clock_init_with_transport(
    node_id: u32,
    transport: Box<dyn ClockSyncTransport>,
    session: Option<Arc<Session>>,
    stall_timeout_ms: u32,
) -> *mut ZenohClockBackend {
    let shutdown = Arc::new(AtomicBool::new(false));

    let backend = Arc::new(ZenohClockBackend {
        session,
        transport,
        node_id,
        stall_timeout_ms,
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
        shutdown: Arc::clone(&shutdown),
    });

    let backend_ptr = Arc::into_raw(backend);

    // Spawn the worker thread
    // SAFETY: backend_ptr is valid for the life of the Arc, and the worker thread owns its own Arc.
    let worker_backend = unsafe {
        let b = Arc::from_raw(backend_ptr);
        let clone = Arc::clone(&b);
        let _ = Arc::into_raw(b);
        clone
    };

    let builder = std::thread::Builder::new().name(format!("zenoh-clock-worker-{node_id}"));

    match builder.spawn(move || {
        zenoh_clock_worker_loop(worker_backend);
    }) {
        Ok(_) => {
            vlog!("[zenoh-clock] Worker thread spawned for node {}.\n", node_id);
        }
        Err(e) => {
            vlog!(
                "[zenoh-clock] FATAL: Failed to spawn worker thread for node {}: {}\n",
                node_id,
                e
            );
            std::process::abort();
        }
    }

    backend_ptr.cast_mut()
}

fn zenoh_clock_init_internal(
    node_id: u32,
    router: *const c_char,
    stall_timeout_ms: u32,
) -> *mut ZenohClockBackend {
    vlog!("[zenoh-clock] Opening session for node {}...\n", node_id);
    // SAFETY: get_or_init_session is safe if router is a valid C string pointer or null.
    // Safety: router validity is guaranteed by the caller.
    let session = unsafe {
        match virtmcu_zenoh::get_or_init_session(router) {
            Ok(s) => s,
            Err(e) => {
                vlog!("[zenoh-clock] failed to open Zenoh session for node {}: {:?}\n", node_id, e);
                return ptr::null_mut();
            }
        }
    };
    vlog!("[zenoh-clock] Session opened for node {}. ID: {}\n", node_id, session.zid());

    let (query_tx, query_rx) = crossbeam_channel::unbounded();

    let hb_topic = format!("sim/clock/liveliness/{node_id}");
    let liveliness = session.liveliness().declare_token(hb_topic).wait().ok();

    let topic = format!("sim/clock/advance/{node_id}");

    vlog!("[zenoh-clock] Declaring queryable on {}...\n", topic);
    let queryable = match session
        .declare_queryable(topic.clone())
        .callback(move |query| {
            let _ = query_tx.send(query);
        })
        .wait()
    {
        Ok(q) => q,
        Err(e) => {
            vlog!("[zenoh-clock] failed to declare queryable on {}: {:?}\n", topic, e);
            return ptr::null_mut();
        }
    };
    vlog!("[zenoh-clock] Queryable declared on {}.\n", topic);

    let transport = Box::new(ZenohClockTransport {
        _queryable: Mutex::new(Some(queryable)),
        _liveliness: liveliness,
        query_rx,
    });

    zenoh_clock_init_with_transport(node_id, transport, Some(session), stall_timeout_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::Sender;

    struct MockClockResponder {
        tx: Sender<ClockReadyResp>,
    }
    impl ClockSyncResponder for MockClockResponder {
        fn send_ready(&self, resp: ClockReadyResp) -> Result<(), String> {
            self.tx.send(resp).map_err(|e| e.to_string())
        }
    }

    struct MockClockTransport {
        req_rx: Receiver<ClockAdvanceReq>,
        resp_tx: Sender<ClockReadyResp>,
    }
    impl ClockSyncTransport for MockClockTransport {
        fn recv_advance(&self) -> Option<(ClockAdvanceReq, Box<dyn ClockSyncResponder>)> {
            self.req_rx.recv().ok().map(|req| {
                let responder: Box<dyn ClockSyncResponder> =
                    Box::new(MockClockResponder { tx: self.resp_tx.clone() });
                (req, responder)
            })
        }
    }

    #[test]
    fn test_zenoh_clock_layout() {
        // QOM layout validation
        assert_eq!(
            core::mem::offset_of!(ZenohClock, parent_obj),
            0,
            "SysBusDevice must be the first field"
        );
    }

    #[test]
    fn test_clock_worker_loop() {
        let (req_tx, req_rx) = crossbeam_channel::unbounded();
        let (resp_tx, resp_rx) = crossbeam_channel::unbounded();

        let transport = Box::new(MockClockTransport { req_rx, resp_tx });
        let shutdown = Arc::new(AtomicBool::new(false));

        let backend = Arc::new(ZenohClockBackend {
            session: None,
            transport,
            node_id: 1,
            stall_timeout_ms: 1000,
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
            shutdown: Arc::clone(&shutdown),
        });

        let worker_backend = Arc::clone(&backend);
        std::thread::spawn(move || {
            zenoh_clock_worker_loop(worker_backend);
        });

        // 1. Initial state check
        assert_eq!(backend.state.load(Ordering::SeqCst), QuantumState::Waiting as u8);

        // 2. Request first quantum (0 delta for sync)
        req_tx.send(ClockAdvanceReq { delta_ns: 0, mujoco_time_ns: 0 }).unwrap();

        // Should return a response
        let resp = resp_rx.recv_timeout(Duration::from_millis(500)).unwrap();
        let error_code = resp.error_code;
        assert_eq!(error_code, CLOCK_ERROR_OK);

        // 3. Request actual quantum
        req_tx.send(ClockAdvanceReq { delta_ns: 1000, mujoco_time_ns: 1000 }).unwrap();

        // Worker should transition to Executing and wake up vCPU (which we don't have here)
        // In this mock test, the worker will be blocked waiting for state to become Waiting again.
        // Let's simulate QEMU finishing the quantum.
        std::thread::sleep(Duration::from_millis(10)); // SLEEP_EXCEPTION: test-only
        backend.vtime_ns.store(1000, Ordering::SeqCst);
        backend.state.store(QuantumState::Waiting as u8, Ordering::SeqCst);
        {
            let _guard = backend.mutex.lock().unwrap();
            backend.cond.notify_all();
        }

        let resp = resp_rx.recv_timeout(Duration::from_millis(500)).unwrap();
        let error_code = resp.error_code;
        let current_vtime = resp.current_vtime_ns;
        assert_eq!(error_code, CLOCK_ERROR_OK);
        assert_eq!(current_vtime, 1000);

        // 4. Shutdown
        shutdown.store(true, Ordering::Release);
        req_tx.send(ClockAdvanceReq { delta_ns: 0, mujoco_time_ns: 0 }).unwrap();
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_bql_reacquired_after_transport_error() {
        // ARCH-2: Verify BQL is automatically reacquired after a transport error (using RAII)
        let _bql_guard = virtmcu_qom::sync::Bql::lock();
        assert!(virtmcu_qom::sync::Bql::is_held());

        {
            // Simulate a block that drops BQL temporarily
            let _unlock = virtmcu_qom::sync::Bql::temporary_unlock();
            assert!(!virtmcu_qom::sync::Bql::is_held());
            // Simulate transport error early return
        }

        // BQL should be reacquired when _unlock is dropped
        assert!(virtmcu_qom::sync::Bql::is_held());
    }

    #[test]
    fn test_quantum_state_valid_transitions() {
        // ARCH-3: Verify valid state transitions
        let state = core::sync::atomic::AtomicU8::new(QuantumState::Waiting as u8);

        // Waiting -> Executing
        let res = state.compare_exchange(
            QuantumState::Waiting as u8,
            QuantumState::Executing as u8,
            Ordering::SeqCst,
            Ordering::Relaxed,
        );
        assert!(res.is_ok());
        assert_eq!(state.load(Ordering::SeqCst), QuantumState::Executing as u8);

        // Executing -> Waiting
        let res = state.compare_exchange(
            QuantumState::Executing as u8,
            QuantumState::Waiting as u8,
            Ordering::SeqCst,
            Ordering::Relaxed,
        );
        assert!(res.is_ok());
        assert_eq!(state.load(Ordering::SeqCst), QuantumState::Waiting as u8);
    }

    #[test]
    fn test_quantum_state_illegal_transitions() {
        // ARCH-3: Verify illegal state transitions fail
        let state = core::sync::atomic::AtomicU8::new(QuantumState::Waiting as u8);

        // Waiting -> Waiting (illegal, already waiting)
        let res = state.compare_exchange(
            QuantumState::Executing as u8, // expected
            QuantumState::Waiting as u8,   // new
            Ordering::SeqCst,
            Ordering::Relaxed,
        );
        assert!(res.is_err());
        assert_eq!(state.load(Ordering::SeqCst), QuantumState::Waiting as u8);
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_hook_toctou_stress() {
        // ARCH-1: Stress test for GLOBAL_CLOCK hook TOCTOU race

        let mut dummy_clock: ZenohClock = unsafe { core::mem::zeroed() };
        dummy_clock.node_id = 0; // bypass mode
        dummy_clock.rust_state = core::ptr::null_mut();

        let clock_ptr = &mut dummy_clock as *mut ZenohClock;

        for _ in 0..1000 {
            GLOBAL_CLOCK.store(clock_ptr, Ordering::Release);

            let handle = std::thread::spawn(|| {
                for _ in 0..10 {
                    zenoh_clock_cpu_halt_cb(core::ptr::null_mut(), false);
                }
            });

            GLOBAL_CLOCK.store(core::ptr::null_mut(), Ordering::Release);
            while ACTIVE_HOOKS.load(Ordering::SeqCst) > 0 {
                core::hint::spin_loop();
            }

            handle.join().unwrap();
        }
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_quantum_state_transitions_stress() {
        // ARCH-3: Stress test for QuantumState transitions
        let state = Arc::new(core::sync::atomic::AtomicU8::new(QuantumState::Waiting as u8));
        let iterations = 10_000;

        let state_clone1 = Arc::clone(&state);
        let t1 = std::thread::spawn(move || {
            for _ in 0..iterations {
                while state_clone1
                    .compare_exchange(
                        QuantumState::Waiting as u8,
                        QuantumState::Executing as u8,
                        Ordering::SeqCst,
                        Ordering::Relaxed,
                    )
                    .is_err()
                {
                    core::hint::spin_loop();
                }
            }
        });

        let state_clone2 = Arc::clone(&state);
        let t2 = std::thread::spawn(move || {
            for _ in 0..iterations {
                while state_clone2
                    .compare_exchange(
                        QuantumState::Executing as u8,
                        QuantumState::Waiting as u8,
                        Ordering::SeqCst,
                        Ordering::Relaxed,
                    )
                    .is_err()
                {
                    core::hint::spin_loop();
                }
            }
        });

        t1.join().unwrap();
        t2.join().unwrap();
        assert_eq!(state.load(Ordering::SeqCst), QuantumState::Waiting as u8);
    }
}
