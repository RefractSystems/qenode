//! Framework for building Co-Simulation Bridges.
//!
//! Provides the `CoSimBridge` IoC container and `CoSimTransport` trait to safely
//! encapsulate QEMU's BQL-yielding wait patterns, eliminating boilerplate and
//! preventing lock-order deadlocks.

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::sim_info;
use crate::sync::{Condvar, Mutex, VcpuDrain};

/// A trait implemented by developers to provide transport-specific co-simulation logic.
pub trait CoSimTransport: Send + Sync {
    /// The type of request sent by the vCPU.
    type Request;
    /// The type of response received from the background loop.
    type Response: Send;

    /// Runs the blocking receive loop in a background thread.
    /// This method is spawned by the framework and should loop as long as `ctx.is_running()` is true.
    /// When a response is parsed, it must be submitted via `ctx.dispatch_response(msg)`.
    fn run_rx_loop(&self, ctx: &CoSimContext<Self::Response>);

    /// Sends a request synchronously.
    /// Called by the framework on behalf of the vCPU thread.
    /// Returns `true` if the request was successfully sent, `false` otherwise.
    fn send_request(&self, req: Self::Request) -> bool;

    /// Interrupts the background receive loop (e.g., closing a socket or unblocking a select).
    /// Called by the framework during device teardown.
    fn interrupt_rx(&self);
}

/// A context provided to the `CoSimTransport::run_rx_loop`.
pub struct CoSimContext<Resp> {
    inner: Arc<BridgeInner<Resp>>,
}

impl<Resp> Clone for CoSimContext<Resp> {
    fn clone(&self) -> Self {
        Self { inner: Arc::clone(&self.inner) }
    }
}

impl<Resp> CoSimContext<Resp> {
    pub(crate) fn new(inner: Arc<BridgeInner<Resp>>) -> Self {
        Self { inner }
    }

    /// Returns `true` if the simulation is currently running and the loop should continue.
    pub fn is_running(&self) -> bool {
        self.inner.running.load(Ordering::Acquire)
    }

    /// Delivers a received response to the blocked vCPU thread.
    pub fn dispatch_response(&self, resp: Resp) {
        let mut guard = self.inner.state.lock();
        guard.current_resp = Some(resp);
        guard.has_resp = true;
        self.inner.resp_cond.notify_all();
    }

    /// Notifies the vCPU thread that the connection has been established.
    pub fn notify_connected(&self) {
        let mut guard = self.inner.state.lock();
        guard.is_connected = true;
        self.inner.connected_cond.notify_all();
    }

    /// Notifies the vCPU thread of a disconnection or error.
    pub fn notify_disconnected(&self) {
        let mut guard = self.inner.state.lock();
        guard.is_connected = false;
        guard.has_resp = true;
        guard.current_resp = None;
        self.inner.resp_cond.notify_all();
        self.inner.connected_cond.notify_all();
        // Wake up any QEMU vCPUs safely.
        unsafe { crate::cpu::virtmcu_cpu_exit_all() };
    }
}

pub(crate) struct BridgeState<Resp> {
    has_resp: bool,
    current_resp: Option<Resp>,
    is_connected: bool,
}

pub(crate) struct BridgeInner<Resp> {
    running: AtomicBool,
    drain: VcpuDrain,
    resp_cond: Condvar,
    connected_cond: Condvar,
    state: Mutex<BridgeState<Resp>>,
}

impl<Resp> BridgeInner<Resp> {
    fn new() -> Self {
        Self {
            running: AtomicBool::new(true),
            drain: VcpuDrain::new(),
            resp_cond: Condvar::new(),
            connected_cond: Condvar::new(),
            state: Mutex::new(BridgeState {
                has_resp: false,
                current_resp: None,
                is_connected: false,
            }),
        }
    }
}

/// The IoC Framework Container for Co-Simulation Bridges.
pub struct CoSimBridge<T: CoSimTransport> {
    transport: Arc<T>,
    inner: Arc<BridgeInner<T::Response>>,
    bg_thread: Option<std::thread::JoinHandle<()>>,
}

impl<T: CoSimTransport + 'static> CoSimBridge<T> {
    /// Creates a new bridge, automatically spawning the transport's receive loop.
    pub fn new(transport: T) -> Self {
        let t = Arc::new(transport);
        let inner = Arc::new(BridgeInner::new());

        let t_clone = Arc::clone(&t);
        let ctx = CoSimContext::new(Arc::clone(&inner));

        let bg_thread = Some(std::thread::spawn(move || {
            t_clone.run_rx_loop(&ctx);
        }));

        Self { transport: t, inner, bg_thread }
    }

    /// Safely sends a request and waits for a response, yielding the BQL automatically.
    /// Returns `None` if the timeout is reached or the bridge is shutting down.
    pub fn send_and_wait(&self, req: T::Request, timeout_ms: u32) -> Option<T::Response> {
        if !self.inner.running.load(Ordering::Acquire) {
            return None;
        }

        // 1. RAII tracking of active vCPUs for safe teardown.
        let _drain_guard = self.inner.drain.acquire();

        // 2. Clear any old responses before sending.
        {
            let mut guard = self.inner.state.lock();
            guard.has_resp = false;
            guard.current_resp = None;
        }

        // 3. Dispatch the request using developer logic.
        if !self.transport.send_request(req) {
            return None;
        }

        // 4. Safely block the vCPU, yielding the BQL to prevent deadlocks.
        let mut guard = self.inner.state.lock();
        let start = std::time::Instant::now();
        let timeout_duration = core::time::Duration::from_millis(timeout_ms as u64);

        while self.inner.running.load(Ordering::Acquire) && !guard.has_resp {
            let elapsed = start.elapsed();
            if elapsed >= timeout_duration {
                sim_info!("timeout waiting for response");
                break;
            }
            let remaining_ms = timeout_duration.checked_sub(elapsed).unwrap().as_millis() as u32;

            // ENTERPRISE SAFETY: Atomically yield BQL, wait, and re-acquire BQL without Lock Inversion.
            let (new_guard, result) = self.inner.resp_cond.wait_yielding_bql(guard, remaining_ms);
            guard = new_guard;

            if !result && start.elapsed() >= timeout_duration {
                sim_info!("timeout waiting for response");
                break;
            }
        }

        guard.current_resp.take()
    }

    /// Waits for the transport to signal it is connected.
    pub fn wait_connected(&self, timeout_ms: u32) -> bool {
        let mut guard = self.inner.state.lock();
        let start = std::time::Instant::now();
        let timeout_duration = core::time::Duration::from_millis(timeout_ms as u64);

        while self.inner.running.load(Ordering::Acquire) && !guard.is_connected {
            let elapsed = start.elapsed();
            if elapsed >= timeout_duration {
                return false;
            }
            let remaining_ms = timeout_duration.checked_sub(elapsed).unwrap().as_millis() as u32;
            let (new_guard, _result) =
                self.inner.connected_cond.wait_yielding_bql(guard, remaining_ms);
            guard = new_guard;
        }
        guard.is_connected
    }
}

impl<T: CoSimTransport> Drop for CoSimBridge<T> {
    fn drop(&mut self) {
        // 1. Signal shutdown to all components.
        self.inner.running.store(false, Ordering::Release);

        // 2. Wake up blocked vCPUs so they can exit `send_and_wait`.
        self.inner.resp_cond.notify_all();
        self.inner.connected_cond.notify_all();

        // 3. Interrupt the transport's blocking syscalls (e.g., socket read).
        self.transport.interrupt_rx();

        // 4. Safely drain vCPUs without deadlocking the main thread (yields BQL).
        // 30 seconds is the bounded timeout to prevent eternal hangs if a vCPU panicked.
        self.inner.drain.wait_for_drain(30_000);

        // 5. Safely join the background thread.
        if let Some(handle) = self.bg_thread.take() {
            // CRITICAL: Background thread might be trying to take BQL.
            // If we don't yield BQL here, we deadlock.
            let _bql = crate::sync::Bql::temporary_unlock();
            let _ = handle.join();
        }
    }
}
