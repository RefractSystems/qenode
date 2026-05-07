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
#![cfg_attr(
    test,
    allow(clippy::expect_used, clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)
)]
#![deny(missing_docs)]
#![doc = "VirtMCU QEMU Object Model (QOM) and System Emulation bindings."]

extern crate alloc;

#[cfg(any(test, miri, feature = "standalone", not(target_os = "none")))]
extern crate std;

/// Character device (Chardev) bindings.
#[macro_export]
/// A macro to count the number of items in a list.
macro_rules! count_items {
    () => { 0 };
    ($head:expr $(, $tail:expr)* $(,)?) => { 1 + $crate::count_items!($($tail),*) };
}

/// Character device (Chardev) bindings.
pub mod chardev;
/// Co-simulation bridging abstractions.
pub mod cosim;
/// CPU-related bindings and hooks.
pub mod cpu;
/// Error handling for QOM operations.
pub mod error;
/// Instruction counting and virtual time advancement.
pub mod icount;
/// Interrupt request (IRQ) and GPIO management.
pub mod irq;
/// Memory region and MMIO management.
pub mod memory;
/// Network device (Netdev) bindings.
pub mod net;
/// Device-level abstractions (DeviceState, SysBusDevice).
pub mod qdev;
/// Core QEMU Object Model (QOM) types and registration.
pub mod qom;
/// Synchronous Serial Interface (SSI/SPI) bindings.
pub mod ssi;
/// Synchronization primitives and Big QEMU Lock (BQL) management.
pub mod sync;
/// General system emulation state and runstate management.
pub mod sysemu;
/// QEMU Timer and virtual clock management.
pub mod timer;

/// Telemetry module
pub mod telemetry;

use core::ffi::c_char;

#[cfg(not(any(test, miri, feature = "standalone", virtmcu_unit_test)))]
extern "C" {
    /// Logs a message to the QEMU/VirtMCU console.
    pub fn virtmcu_log(fmt: *const c_char);

    /// Returns the size of the QEMU `DeviceState` struct.
    pub fn virtmcu_sizeof_device_state() -> usize;
    /// Returns the size of the QEMU `SysBusDevice` struct.
    pub fn virtmcu_sizeof_sys_bus_device() -> usize;
    /// Returns the size of the QEMU `DeviceClass` struct.
    pub fn virtmcu_sizeof_device_class() -> usize;
    /// Returns the size of the QEMU `SSIPeripheral` struct.
    pub fn virtmcu_sizeof_ssi_peripheral() -> usize;
    /// Returns the size of the QEMU `SSIPeripheralClass` struct.
    pub fn virtmcu_sizeof_ssi_peripheral_class() -> usize;
    /// Returns the size of the QEMU `Chardev` struct.
    pub fn virtmcu_sizeof_chardev() -> usize;
    /// Returns the size of the QEMU `ChardevClass` struct.
    pub fn virtmcu_sizeof_chardev_class() -> usize;
    /// Returns the size of the QEMU `CharBackend` struct.
    pub fn virtmcu_sizeof_char_backend() -> usize;
}

#[cfg(any(test, miri, feature = "standalone", virtmcu_unit_test))]
#[no_mangle]
/// Stub for virtmcu_log in tests and standalone mode.
pub unsafe extern "C" fn virtmcu_log(_fmt: *const c_char) {}
#[cfg(any(test, miri, feature = "standalone", virtmcu_unit_test))]
#[no_mangle]
/// Returns a stub size for DeviceState in tests.
pub extern "C" fn virtmcu_sizeof_device_state() -> usize {
    1024
}
#[cfg(any(test, miri, feature = "standalone", virtmcu_unit_test))]
#[no_mangle]
/// Returns a stub size for SysBusDevice in tests.
pub extern "C" fn virtmcu_sizeof_sys_bus_device() -> usize {
    1024
}
#[cfg(any(test, miri, feature = "standalone", virtmcu_unit_test))]
#[no_mangle]
/// Returns a stub size for DeviceClass in tests.
pub extern "C" fn virtmcu_sizeof_device_class() -> usize {
    1024
}
#[cfg(any(test, miri, feature = "standalone", virtmcu_unit_test))]
#[no_mangle]
/// Returns a stub size for SSIPeripheral in tests.
pub extern "C" fn virtmcu_sizeof_ssi_peripheral() -> usize {
    1024
}
#[cfg(any(test, miri, feature = "standalone", virtmcu_unit_test))]
#[no_mangle]
/// Returns a stub size for SSIPeripheralClass in tests.
pub extern "C" fn virtmcu_sizeof_ssi_peripheral_class() -> usize {
    1024
}
#[cfg(any(test, miri, feature = "standalone", virtmcu_unit_test))]
#[no_mangle]
/// Returns a stub size for Chardev in tests.
pub extern "C" fn virtmcu_sizeof_chardev() -> usize {
    1024
}
#[cfg(any(test, miri, feature = "standalone", virtmcu_unit_test))]
#[no_mangle]
/// Returns a stub size for ChardevClass in tests.
pub extern "C" fn virtmcu_sizeof_chardev_class() -> usize {
    1024
}
#[cfg(any(test, miri, feature = "standalone", virtmcu_unit_test))]
#[no_mangle]
/// Returns a stub size for CharBackend in tests.
pub extern "C" fn virtmcu_sizeof_char_backend() -> usize {
    1024
}

#[macro_export]
/// Logs a formatted message to the VirtMCU log using an internal buffer.
macro_rules! vlog {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut buf = [0u8; 1024];
        let mut cursor = $crate::BufCursor::new(&mut buf);
        let _ = write!(cursor, $($arg)*);
        let _ = write!(cursor, "\0");
        // SAFETY: virtmcu_log takes a null-terminated string. buf contains a null-terminated
        // string at this point. The buffer is alive for the duration of the call.
        unsafe { $crate::virtmcu_log(buf.as_ptr() as *const _) };
    }};
}

/// A simple stack-allocated cursor for writing to a byte buffer.
pub struct BufCursor<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl<'a> BufCursor<'a> {
    /// Creates a new `BufCursor` wrapping the provided buffer.
    pub fn pos(&self) -> usize {
        self.pos
    }

    /// Create a new BufCursor
    pub fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, pos: 0 }
    }
}

impl core::fmt::Write for BufCursor<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let bytes = s.as_bytes();
        let mut len = bytes.len();
        let mut truncated = false;

        if self.pos + len > self.buf.len() {
            len = self.buf.len() - self.pos;
            truncated = true;
        }

        if len > 0 {
            if let (Some(dest), Some(src)) =
                (self.buf.get_mut(self.pos..self.pos + len), bytes.get(..len))
            {
                dest.copy_from_slice(src);
                self.pos += len;
            }
        }

        if truncated {
            Err(core::fmt::Error)
        } else {
            Ok(())
        }
    }
}
