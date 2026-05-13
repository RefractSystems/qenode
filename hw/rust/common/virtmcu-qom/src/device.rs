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
    },
}

impl<'a> MmioResult<'a> {
    /// Helper to construct a `Wait` result with a condition and a ready_val closure.
    pub fn wait_for<C, R>(condition: C, ready_val: R) -> Self
    where
        C: FnMut() -> bool + 'a,
        R: FnMut() -> u64 + 'a,
    {
        MmioResult::Wait { condition: Box::new(condition), ready_val: Box::new(ready_val) }
    }
}

/// A safe trait for developing QEMU MMIO peripherals.
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
