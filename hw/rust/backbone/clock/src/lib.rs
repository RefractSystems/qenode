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
//! Virtmcu deterministic clock with pluggable transport.
//!
//! This module provides the `VirtmcuClock` QOM device, which synchronizes
//! the guest's virtual time with an external Physical Node.

extern crate alloc;

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use core::ffi::{c_char, c_void, CStr};
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use core::time::Duration;
use crossbeam_channel::Receiver;
use std::collections::HashMap;
use std::sync::{Condvar, Mutex};
use std::time::Instant;

#[derive(Clone, Copy)]
struct ClockPtr(*mut VirtmcuClock);
unsafe impl Send for ClockPtr {}
unsafe impl Sync for ClockPtr {}

static CLOCK_REGISTRY: Mutex<Option<HashMap<u32, ClockPtr>>> = Mutex::new(None); // virtmcu-allow: static_state reasoning="Required for C-FFI hook dispatch"

use virtmcu_api::{
    ClockAdvanceReq, ClockReadyResp, ClockSyncResponder, ClockSyncTransport, FlatBufferStructExt,
    BOOT_QUANTUM_TIMEOUT, CLOCK_ERROR_OK, CLOCK_ERROR_STALL, NORMAL_QUANTUM_TIMEOUT,
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
    _queryable: Mutex<Option<Queryable<()>>>, // virtmcu-allow: mutex reasoning="used for thread shutdown synchronization"
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

const CLOCK_ADVANCE_REQ_SIZE: usize = core::mem::size_of::<ClockAdvanceReq>();

impl ClockSyncTransport for ZenohClockTransport {
    fn recv_advance(
        &self,
        timeout: core::time::Duration,
    ) -> Option<(ClockAdvanceReq, Box<dyn ClockSyncResponder>)> {
        match self.query_rx.recv_timeout(timeout) {
            Ok(query) => {
                let data = query.payload().map(|p| p.to_bytes()).unwrap_or_default();
                match ClockAdvanceReq::unpack_slice(&data) {
                    Some(req) => {
                        let responder: Box<dyn ClockSyncResponder> =
                            Box::new(ZenohClockResponder {
                                query,
                                start_rx: self.start_rx.clone(),
                                done_pub: alloc::sync::Arc::clone(&self.done_pub),
                                quantum: req.quantum_number(),
                                is_coordinated: self.is_coordinated,
                            });
                        Some((req, responder))
                    }
                    None => {
                        virtmcu_qom::sim_err!(
                            "ZenohClockTransport: Received malformed ClockAdvanceReq (size={}, expected {}). Ensure your Physical Node uses the {}-byte protocol.",
                            data.len(),
                            CLOCK_ADVANCE_REQ_SIZE,
                            CLOCK_ADVANCE_REQ_SIZE
                        );
                        None
                    }
                }
            }
            Err(_) => None,
        }
    }

    fn send_vtime_heartbeat(&self, vtime_ns: u64) {
        let mut payload = alloc::vec::Vec::new();
        payload.extend_from_slice(&vtime_ns.to_le_bytes());
        self.vtime_pub.send(payload);
    }
}

impl Drop for ZenohClockTransport {
    fn drop(&mut self) {
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

const CLOCK_RESP_PAYLOAD_SIZE: usize = 16;
const DRAIN_WAIT_TIMEOUT_MS: u64 = 100;
const CLOCK_EXECUTING_TIMEOUT_MS: u64 = 100;
const CLOCK_DEFAULT_WATCHDOG_THRESHOLD: u64 = 3;

impl ClockSyncResponder for ZenohClockResponder {
    fn send_ready(&self, resp: ClockReadyResp) -> Result<(), String> {
        // Only the Zenoh coordinated path waits for sim/clock/start before
        // advancing the first quantum. Unix-mode federates never reach this branch.
        if self.is_coordinated {
            // 1. Send 'done' signal to coordinator: [quantum (8), current_vtime_ns (8)]
            let mut payload = alloc::vec::Vec::with_capacity(CLOCK_RESP_PAYLOAD_SIZE);
            payload.extend_from_slice(&self.quantum.to_le_bytes());
            payload.extend_from_slice(&resp.current_vtime_ns().to_le_bytes());
            self.done_pub.send(payload);

            // 2. Wait for 'start' signal from coordinator
            if let Err(e) = self.start_rx.recv() {
                virtmcu_qom::sim_err!(
                    "start_rx channel disconnected before receiving start signal: {}",
                    e
                );
            }
        }

        // 3. Release the reply back to the Physical Node
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
    /// Identifier for this running simulation instance.
    pub federation_id: *mut c_char,
    /// Synchronization mode ("slaved-suspend", "slaved-icount", "unix").
    pub mode: *mut c_char,
    /// Optional router address or socket path.
    pub router: *mut c_char,
    /// Timeout in milliseconds before a clock stall is declared.
    pub stall_timeout: u32,
    /// Whether to synchronize with a Deterministic Coordinator.
    pub coordinated: bool,
    pub session_watchdog_ms: u32,
    pub debug: bool,
    pub n_vcpus: u32,

    /* Internal State */
    /// Virtual time (ns) of the next quantum boundary.
    pub next_quantum_ns: i64,
    /// Virtual time (ns) of the last halt event.
    pub last_halt_vtime: i64,
    /// Timer used to trigger quantum boundary checks.
    pub quantum_timer: *mut QemuTimer,

    /* Rust state */
    /// Opaque pointer to the Rust manager state.
    pub rust_state: *mut ClockManager,
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

const CLOCK_WAIT_TIMEOUT_MS: u64 = 10;

/// Internal Rust backend for `VirtmcuClock`.
pub struct VirtmcuClockBackend {
    pub n_vcpus: core::sync::atomic::AtomicU32,
    pub vcpu_halt_count: core::sync::atomic::AtomicU32,
    /// Optional Zenoh session for communication.
    pub session: Option<Arc<Session>>,
    /// Abstract transport for clock synchronization.
    pub transport: Box<dyn ClockSyncTransport>,
    /// Unique node ID.
    pub node_id: u32,
    /// Identifier for this running simulation instance.
    pub federation_id: virtmcu_api::FederationId,
    /// Stall timeout in milliseconds.
    pub stall_timeout_ms: u32,
    /// Whether coordination is enabled for this node.
    pub is_coordinated: bool,
    pub watchdog_threshold: u64,
    pub consecutive_timeouts: core::sync::atomic::AtomicU64,
    pub abort_fn: alloc::sync::Arc<dyn Fn() + Send + Sync>,

    /* Communication state */
    /// Mutex for protecting communication state.
    pub mutex: Mutex<()>, // virtmcu-allow: mutex reasoning="used with Condvar for cross-thread sync (vCPU <-> Worker)"
    /// Condvar for signaling quantum events.
    pub cond: Condvar,

    /// Explicit state machine for quantum synchronization.
    pub state: core::sync::atomic::AtomicU8,
    /// Number of nanoseconds to advance in the current/next quantum.
    pub delta_ns: AtomicU64,
    /// Current virtual time in nanoseconds as known by the backend.
    pub vtime_ns: AtomicU64,
    /// Absolute simulation time in nanoseconds as reported by Physical Node.
    pub absolute_vtime_ns: AtomicU64,
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

/// A RAII manager that owns the worker thread and the shared backend state.
pub struct ClockManager {
    pub backend: Arc<VirtmcuClockBackend>,
    pub worker_thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for ClockManager {
    fn drop(&mut self) {
        // 1. Set running = false (shutdown flag)
        self.backend.shutdown.store(true, Ordering::Release);

        // 2. Broadcast condvars
        let mut guard =
            self.backend.mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        self.backend.cond.notify_all();

        // 3. Wait via drain_cond until active_vcpu_count == 0
        while ACTIVE_HOOKS.load(Ordering::SeqCst) > 0 {
            let bql_unlock = virtmcu_qom::sync::Bql::temporary_unlock();
            let (new_guard, _) = self
                .backend
                .cond
                .wait_timeout(guard, core::time::Duration::from_millis(DRAIN_WAIT_TIMEOUT_MS))
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard = new_guard;
            drop(bql_unlock);
        }
        drop(guard);

        // 4. Join background thread
        if let Some(thread) = self.worker_thread.take() {
            let _ = thread.join();
        }

        // 5. Arc<SharedState> is implicitly dropped here.
    }
}

/* ── Logic ────────────────────────────────────────────────────────────────── */

static ACTIVE_HOOKS: AtomicU64 = AtomicU64::new(0); // virtmcu-allow: static_state reasoning="Required for C-FFI hook dispatch"

extern "C" {
    fn virtmcu_vcpu_should_yield() -> bool;
}

extern "C" fn clock_quantum_timer_cb(_opaque: *mut c_void) {
    // SAFETY: called from a QEMU timer callback; BQL is held by the QEMU main loop.
    // Kick all CPUs to ensure they exit their TCG loops and reach the boundary sync point.
    unsafe { virtmcu_qom::cpu::virtmcu_cpu_exit_all() };
}

extern "C" fn clock_cpu_tcg_hook(cpu: *mut CPUState) {
    let _guard = ActiveHooksGuard::new();
    let mut clocks = Vec::new();
    {
        if let Ok(lock) = CLOCK_REGISTRY.lock() {
            if let Some(map) = &*lock {
                for ptr in map.values() {
                    clocks.push(ptr.0);
                }
            }
        }
    }
    for s_ptr in clocks {
        if !s_ptr.is_null() {
            let s = unsafe { &mut *s_ptr };
            if !s.rust_state.is_null() {
                clock_cpu_halt_cb_internal(s, cpu, false);
            }
        }
    }
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

        let mut clocks = Vec::new();
        {
            if let Ok(lock) = CLOCK_REGISTRY.lock() {
                if let Some(map) = &*lock {
                    for ptr in map.values() {
                        clocks.push(ptr.0);
                    }
                }
            }
        }

        for s_ptr in clocks {
            if !s_ptr.is_null() {
                // SAFETY: s_ptr is checked for null and is a valid pointer to VirtmcuClock when not null.
                let s = unsafe { &mut *s_ptr };
                if !s.rust_state.is_null() {
                    clock_cpu_halt_cb_internal(s, _cpu, halted);
                }
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

    if s.debug {
        virtmcu_qom::sim_info!("Clock: CPU hook triggered (halted={})", halted);
    }

    // SAFETY: Calling qemu_clock_get_ns is safe under BQL or from vCPU thread.
    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };
    virtmcu_qom::telemetry::update_global_vtime(u64::try_from(now).expect("vtime is negative"));

    if now >= s.next_quantum_ns || halted {
        if s.rust_state.is_null() {
            return;
        }
        let manager = unsafe { &*s.rust_state };
        let backend = &manager.backend;

        // Release BQL before blocking using RAII guard
        let bql_unlock = virtmcu_qom::sync::Bql::temporary_unlock();

        let raw_delta = clock_quantum_wait_internal(
            backend,
            u64::try_from(now).expect("vtime is negative"),
            s.is_yielding,
        );
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
            let bql_wait = u64::try_from(bql_start.elapsed().as_nanos()).expect("nanos truncated");
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
    _is_yielding: bool,
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

    let n_vcpus = backend.n_vcpus.load(Ordering::SeqCst);
    let count = backend.vcpu_halt_count.fetch_add(1, Ordering::SeqCst) + 1;

    backend.vtime_ns.store(_vtime_ns, Ordering::SeqCst);

    if count >= n_vcpus {
        // All vCPUs have arrived at the barrier.
        // Transition: Executing -> Waiting
        let current = backend.state.load(Ordering::Acquire);
        if current == QuantumState::Executing as u8 {
            backend.state.store(QuantumState::Waiting as u8, Ordering::SeqCst);
        }

        // Notify TA that we finished previous quantum
        {
            let _guard = backend.mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            backend.cond.notify_all();
        }
    }

    // Stage 1: Barrier. Wait until ALL vCPUs have arrived and state transitioned to Waiting.
    // If state is still Executing, it means other vCPUs are still running.
    {
        let mut guard = backend.mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        while backend.state.load(Ordering::SeqCst) == QuantumState::Executing as u8 {
            if backend.shutdown.load(Ordering::Acquire) {
                return 0;
            }
            if unsafe { virtmcu_vcpu_should_yield() } {
                backend.vcpu_halt_count.fetch_sub(1, Ordering::SeqCst);
                return QUANTUM_WAIT_YIELD_SENTINEL;
            }
            let (new_guard, _) = backend
                .cond
                .wait_timeout(guard, Duration::from_millis(CLOCK_WAIT_TIMEOUT_MS))
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard = new_guard;
        }
    }

    let start = Instant::now();
    let is_first = backend.is_first_quantum.load(Ordering::Relaxed);
    let timeout = if is_first {
        BOOT_QUANTUM_TIMEOUT
    } else {
        Duration::from_millis(u64::from(backend.stall_timeout_ms))
    };

    // Stage 2: Wait for TA to grant the next quantum (transition to Executing).
    {
        let mut guard = backend.mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        while backend.state.load(Ordering::SeqCst) != QuantumState::Executing as u8 {
            if backend.shutdown.load(Ordering::Acquire) {
                return 0;
            }
            if unsafe { virtmcu_vcpu_should_yield() } {
                backend.vcpu_halt_count.fetch_sub(1, Ordering::SeqCst);
                return QUANTUM_WAIT_YIELD_SENTINEL;
            }

            // Wait for Executing
            let (new_guard, result) = backend
                .cond
                .wait_timeout(guard, Duration::from_millis(CLOCK_EXECUTING_TIMEOUT_MS))
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
                if backend.is_coordinated {
                    virtmcu_qom::sim_info!(
                        "[{}] Received advance for quantum {} (delta={}ns, abs={}ns)",
                        backend.federation_id,
                        r.0.quantum_number(),
                        r.0.delta_ns(),
                        r.0.absolute_vtime_ns()
                    );
                }
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
        let absolute_time = req.absolute_vtime_ns();

        backend.delta_ns.store(delta, Ordering::SeqCst);
        backend.absolute_vtime_ns.store(absolute_time, Ordering::SeqCst);

        let mut error_code = wait_for_ready_and_execute(&backend, delta, timeout, is_first);

        if backend.pending_stall.swap(false, Ordering::SeqCst) {
            error_code = CLOCK_ERROR_STALL;
        }

        let current_vtime = backend.vtime_ns.load(Ordering::SeqCst);
        backend.transport.send_vtime_heartbeat(current_vtime);

        if backend.is_coordinated {
            virtmcu_qom::sim_info!(
                "[{}] Sending ready for quantum {} (vtime={}ns, error={})",
                backend.federation_id,
                req.quantum_number(),
                current_vtime,
                error_code
            );
        }

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
    _is_first: bool,
) -> u32 {
    let start = Instant::now();

    {
        let mut guard = backend.mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        while backend.state.load(Ordering::SeqCst) != QuantumState::Waiting as u8 {
            if backend.shutdown.load(Ordering::Acquire) {
                return CLOCK_ERROR_OK;
            }
            if start.elapsed() > timeout {
                return CLOCK_ERROR_STALL;
            }

            let (new_guard, _) = backend
                .cond
                .wait_timeout(guard, Duration::from_millis(CLOCK_WAIT_TIMEOUT_MS))
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard = new_guard;
        }
    }

    virtmcu_qom::sim_info!("Clock: Worker granting quantum ({} ns)", delta);
    // Reset barrier for the next quantum
    backend.vcpu_halt_count.store(0, Ordering::SeqCst);

    {
        let _guard = backend.mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let current = QuantumState::from(backend.state.load(Ordering::Acquire));
        if current == QuantumState::Waiting {
            backend.state.store(QuantumState::Executing as u8, Ordering::SeqCst);
            backend.cond.notify_all();
        } else {
            virtmcu_qom::sim_warn!(
                "Clock: Worker attempted to grant quantum but state is {:?}",
                current
            );
        }
    }

    if delta > 0 {
        let exec_start = Instant::now();
        let mut guard = backend.mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        while backend.state.load(Ordering::SeqCst) != QuantumState::Waiting as u8 {
            if backend.shutdown.load(Ordering::Acquire) {
                return CLOCK_ERROR_OK;
            }
            if exec_start.elapsed() > timeout {
                return CLOCK_ERROR_STALL;
            }

            let (new_guard, _) = backend
                .cond
                .wait_timeout(guard, Duration::from_millis(CLOCK_WAIT_TIMEOUT_MS))
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard = new_guard;
        }
    }

    let is_first = backend.is_first_quantum.load(Ordering::Relaxed);
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
        const CONTENTION_PERCENT_SCALE: f64 = 100.0;
        const CONTENTION_PRECISION: usize = 2;
        let contention =
            (total_wait as f64 / (elapsed * 1_000_000_000.0)) * CONTENTION_PERCENT_SCALE;
        virtmcu_qom::sim_info!(
            "{:.*}% (samples: {}, no_bql: {})",
            CONTENTION_PRECISION,
            contention,
            iterations,
            no_bql
        );
    }
    *last_report = Instant::now();
}

/// Realise the virtmcu-clock QOM device and select a transport.
///
/// # Transport dispatch
///
/// | `mode` parameter   | Transport                    | `sim/clock/start` needed? |
/// |--------------------|------------------------------|---------------------------|
/// | `standalone`       | None (QEMU free-runs)        | No                        |
/// | `slaved-unix`      | `UnixSocketClockTransport`   | **No** — exits via `clock_init_with_transport()`, Zenoh code never runs |
/// | `slaved-suspend`   | `ZenohClockTransport`        | Only if `is_coordinated=true` |
/// | `slaved-icount`    | `ZenohClockTransport`        | Only if `is_coordinated=true` |
///
/// The `sim/clock/start` Zenoh topic is only relevant in the Zenoh path and
/// only when `is_coordinated = true`. If you are using `mode=slaved-unix`, do
/// not send or wait for `sim/clock/start` — it has no effect.
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
        stall_ms = u32::try_from(NORMAL_QUANTUM_TIMEOUT.as_millis()).expect("millis truncated");
    }

    let is_coordinated = s.coordinated;
    let n_vcpus = s.n_vcpus;

    let fed_id_str = if s.federation_id.is_null() {
        "unnamed-federation"
    } else {
        unsafe { CStr::from_ptr(s.federation_id) }.to_str().unwrap_or("unnamed-federation")
    };
    let federation_id = virtmcu_api::FederationId(fed_id_str.to_owned());

    if is_unix {
        if router_str.is_null() {
            virtmcu_qom::error_setg!(errp, "clock: 'router' (socket path) required for unix\n");
            return;
        }
        let path = unsafe { CStr::from_ptr(router_str) }.to_string_lossy();
        let transport = virtmcu_api::UnixSocketClockTransport::new(path.as_ref());
        let watchdog_ms = if s.session_watchdog_ms > 0 {
            s.session_watchdog_ms
        } else {
            stall_ms * CLOCK_DEFAULT_WATCHDOG_THRESHOLD as u32
        };
        s.rust_state = clock_init_with_transport(ClockManagerConfig {
            node_id: s.node_id,
            n_vcpus,
            transport: Box::new(transport),
            session: None,
            stall_timeout_ms: stall_ms,
            is_coordinated,
            session_watchdog_ms: watchdog_ms,
            federation_id,
        });
    } else {
        let watchdog_ms = if s.session_watchdog_ms > 0 {
            s.session_watchdog_ms
        } else {
            stall_ms * CLOCK_DEFAULT_WATCHDOG_THRESHOLD as u32
        };
        s.rust_state = clock_init_internal(
            s.node_id,
            n_vcpus,
            router_str,
            stall_ms,
            is_coordinated,
            watchdog_ms,
            federation_id,
        );
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

    {
        let mut lock = CLOCK_REGISTRY.lock().expect("Registry lock failed");
        let map = lock.get_or_insert_with(HashMap::new);
        map.insert(s.node_id, ClockPtr(s));
    }

    unsafe {
        virtmcu_qom::cpu::virtmcu_cpu_set_halt_hook(Some(clock_cpu_halt_cb));
        virtmcu_qom::cpu::virtmcu_cpu_set_tcg_hook(Some(clock_cpu_tcg_hook));
    }
}

unsafe extern "C" fn clock_instance_finalize(obj: *mut Object) {
    let s = &mut *(obj as *mut VirtmcuClock);
    {
        let mut lock = CLOCK_REGISTRY.lock().expect("Registry lock failed");
        if let Some(map) = &mut *lock {
            map.remove(&s.node_id);
        }
    }
    unsafe {
        virtmcu_qom::cpu::virtmcu_cpu_set_halt_hook(None);
        virtmcu_qom::cpu::virtmcu_cpu_set_tcg_hook(None);
    }

    if !s.rust_state.is_null() {
        // Construct Box to trigger the Drop implementation
        let _manager = unsafe { Box::from_raw(s.rust_state) };
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
    s.n_vcpus = 1;
}

define_properties!(
    VIRT_CLOCK_PROPERTIES,
    [
        define_prop_uint32!(c"node".as_ptr(), VirtmcuClock, node_id, 0xFFFF_FFFF),
        define_prop_string!(c"federation_id".as_ptr(), VirtmcuClock, federation_id),
        define_prop_uint32!(c"n-vcpus".as_ptr(), VirtmcuClock, n_vcpus, 1),
        define_prop_string!(c"mode".as_ptr(), VirtmcuClock, mode),
        define_prop_string!(c"router".as_ptr(), VirtmcuClock, router),
        define_prop_uint32!(c"stall-timeout".as_ptr(), VirtmcuClock, stall_timeout, 0),
        define_prop_bool!(c"coordinated".as_ptr(), VirtmcuClock, coordinated, false),
        define_prop_uint32!(c"session-watchdog-ms".as_ptr(), VirtmcuClock, session_watchdog_ms, 0),
        virtmcu_qom::define_prop_bool!(c"debug".as_ptr(), VirtmcuClock, debug, false),
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

#[used]
static VIRT_CLOCK_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"virtmcu-clock".as_ptr(),
    parent: c"sys-bus-device".as_ptr(),
    instance_size: core::mem::size_of::<VirtmcuClock>(),
    instance_align: 0,
    instance_init: Some(clock_instance_init),
    instance_post_init: None,
    instance_finalize: Some(clock_instance_finalize),
    abstract_: false,
    class_size: core::mem::size_of::<virtmcu_qom::qdev::SysBusDeviceClass>(),
    class_init: Some(clock_class_init),
    class_base_init: None,
    class_data: ptr::null(),
    interfaces: ptr::null(),
};

declare_device_type!(VIRT_CLOCK_TYPE_INIT, VIRT_CLOCK_TYPE_INFO);

struct ClockManagerConfig {
    node_id: u32,
    n_vcpus: u32,
    transport: Box<dyn ClockSyncTransport>,
    session: Option<Arc<Session>>,
    stall_timeout_ms: u32,
    is_coordinated: bool,
    session_watchdog_ms: u32,
    federation_id: virtmcu_api::FederationId,
}

fn clock_init_with_transport(config: ClockManagerConfig) -> *mut ClockManager {
    let shutdown = Arc::new(AtomicBool::new(false));
    let watchdog_threshold = if config.session_watchdog_ms > 0 && config.stall_timeout_ms > 0 {
        (config.session_watchdog_ms / config.stall_timeout_ms) as u64
    } else {
        CLOCK_DEFAULT_WATCHDOG_THRESHOLD
    };
    let backend = Arc::new(VirtmcuClockBackend {
        n_vcpus: core::sync::atomic::AtomicU32::new(config.n_vcpus),
        vcpu_halt_count: core::sync::atomic::AtomicU32::new(0),
        session: config.session,
        transport: config.transport,
        node_id: config.node_id,
        federation_id: config.federation_id,
        stall_timeout_ms: config.stall_timeout_ms,
        is_coordinated: config.is_coordinated,
        watchdog_threshold,
        consecutive_timeouts: core::sync::atomic::AtomicU64::new(0),
        abort_fn: Arc::new(|| std::process::exit(1)),
        mutex: Mutex::new(()),
        cond: Condvar::new(),
        state: core::sync::atomic::AtomicU8::new(QuantumState::Waiting as u8),
        delta_ns: AtomicU64::new(0),
        vtime_ns: AtomicU64::new(0),
        absolute_vtime_ns: AtomicU64::new(0),
        stall_count: AtomicU64::new(0),
        total_bql_wait_ns: AtomicU64::new(0),
        total_iterations: AtomicU64::new(0),
        total_no_bql_iterations: AtomicU64::new(0),
        start_time: Instant::now(),
        is_first_quantum: AtomicBool::new(true),
        pending_stall: AtomicBool::new(false),
        shutdown: Arc::clone(&shutdown),
    });

    let worker_backend = Arc::clone(&backend);
    let worker_thread = Some(std::thread::spawn(move || clock_worker_loop(worker_backend)));

    let manager = Box::new(ClockManager { backend, worker_thread });

    Box::into_raw(manager)
}

fn clock_init_internal(
    node_id: u32,
    n_vcpus: u32,
    router: *const c_char,
    stall_timeout_ms: u32,
    is_coordinated: bool,
    session_watchdog_ms: u32,
    federation_id: virtmcu_api::FederationId,
) -> *mut ClockManager {
    let session = unsafe {
        let mut config = match transport_zenoh::open_config(router) {
            Ok(c) => c,
            Err(_) => return ptr::null_mut(),
        };
        let _ = config
            .insert_json5("metadata/federation_id", &format!("\"{}\"", federation_id.as_str()));
        match zenoh::open(config).wait() {
            Ok(s) => Arc::new(s),
            Err(_) => return ptr::null_mut(),
        }
    };
    let (query_tx, query_rx) = crossbeam_channel::unbounded();
    let (start_tx, start_rx) = crossbeam_channel::unbounded();
    let start_topic = format!("sim/clock/start/{node_id}");
    let start_transport = Arc::new(transport_zenoh::ZenohDataTransport::new(Arc::clone(&session)));
    // NOTE: `sim/clock/start` is only subscribed here when `is_coordinated = true`.
    // The Unix path (`slaved-unix`) never reaches this function — it exits via
    // `clock_init_with_transport()` in `clock_realize()`. Operators running
    // `slaved-unix` mode should NOT include `sim/clock/start` signals in their
    // world configuration; they have no effect and will mislead operators.
    let _start_sub = virtmcu_qom::sync::SafeSubscription::new(
        start_transport.as_ref(),
        &start_topic,
        Arc::new(AtomicU64::new(0)),
        Box::new(move |_topic: &str, _payload: &[u8]| {
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
    clock_init_with_transport(ClockManagerConfig {
        node_id,
        n_vcpus,
        transport,
        session: Some(session),
        stall_timeout_ms,
        is_coordinated,
        session_watchdog_ms,
        federation_id,
    })
}

#[cfg(test)]
#[allow(clippy::magic_numbers)] // virtmcu-allow: allow reasoning="Tests require specific magic numbers"
mod tests {
    use super::*;
    #[test]
    fn test_clock_layout() {
        assert_eq!(core::mem::offset_of!(VirtmcuClock, parent_obj), 0);
    }
}
