use crate::sync::{Condvar, Mutex};
use alloc::boxed::Box;

/// The result of an MMIO read operation.
pub enum MmioResult<'a> {
    /// The value is ready to be returned immediately.
    Ready(u64),
    /// The peripheral is waiting for an asynchronous event (e.g. Zenoh packet).
    /// The infrastructure will yield the BQL and wait on the condition variable.
    Wait {
        /// The condition to check. Returning true means the event has occurred.
        condition: Box<dyn FnMut() -> bool + 'a>,
        /// The closure that yields the final register value once the condition is met.
        ready_val: Box<dyn FnMut() -> u64 + 'a>,
        /// The closure that yields the fallback register value if the condition is not met yet
        /// (used when icount is active and blocking is fatal).
        fallback_val: Box<dyn FnMut() -> u64 + 'a>,
    },
}

impl<'a> MmioResult<'a> {
    /// Helper to construct a `Wait` result with a condition and a ready_val closure.
    pub fn wait_for<C, R, F>(condition: C, ready_val: R, fallback_val: F) -> Self
    where
        C: FnMut() -> bool + 'a,
        R: FnMut() -> u64 + 'a,
        F: FnMut() -> u64 + 'a,
    {
        MmioResult::Wait {
            condition: Box::new(condition),
            ready_val: Box::new(ready_val),
            fallback_val: Box::new(fallback_val),
        }
    }
}

/// A zero-cost token proving that the VcpuDrain lock is held.
/// This prevents developers from accidentally calling MMIO methods
/// without acquiring the drain guard, or from nested deadlocks.
pub struct DrainToken {
    _private: (),
}

impl DrainToken {
    /// Internal: Create a new token. This must only be called by the framework
    /// after acquiring a VcpuDrain guard.
    #[doc(hidden)]
    pub unsafe fn new_unchecked() -> Self {
        Self { _private: () }
    }
}

/// The base trait for peripheral state management (RFC-0023).
pub trait PeripheralState {
    /// The associated QOM FFI struct type.
    type QomType;

    /// Create a new state instance from the QOM FFI struct.
    /// This is where you pull properties and initialize safe resources.
    fn new(qemu_dev: &Self::QomType) -> Self;
}

/// The unified trait for all VirtMCU peripherals (RFC-0023).
///
/// This replaces the legacy split-brain `MmioDevice` pattern. Implementers
/// focus on safe business logic while the framework handles the
/// QOM/FFI boundary shims and implicit synchronization.
pub trait Peripheral {
    /// Called during QEMU's realize phase.
    /// Use this to initialize subscriptions, timers, and dependencies.
    fn realize(&mut self) -> Result<(), alloc::string::String> {
        Ok(())
    }

    /// Handles an MMIO read request.
    /// The presence of the `DrainToken` proves the BQL and VcpuDrain are correctly held.
    fn read(&self, offset: u64, size: u32, token: &DrainToken) -> MmioResult<'_>;

    /// Handles an MMIO write request.
    /// The presence of the `DrainToken` proves the BQL and VcpuDrain are correctly held.
    fn write(&self, offset: u64, value: u64, size: u32, token: &DrainToken);

    /// Called during QEMU machine reset.
    fn reset(&mut self) {}

    /// Returns the condition variable used for `wait_yielding_bql`.
    fn condvar(&self) -> &Condvar;

    /// Returns the mutex guarding the condition variable wait.
    fn wait_mutex(&self) -> &Mutex<()>;
}

/// A safe trait for developing QEMU MMIO peripherals.
/// DEPRECATED: Use the unified `Peripheral` trait from RFC-0023 instead.
pub trait MmioDevice {
    /// Handles an MMIO read request at the given relative offset.
    fn read(&self, offset: u64, size: u32) -> MmioResult<'_>;

    /// Handles an MMIO write request at the given relative offset with a value.
    fn write(&self, offset: u64, value: u64, size: u32);

    /// Returns the condition variable used for `wait_yielding_bql`.
    fn condvar(&self) -> &Condvar;

    /// Returns the mutex guarding the condition variable wait.
    fn wait_mutex(&self) -> &Mutex<()>;
}
