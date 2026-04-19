#![allow(
    clippy::missing_safety_doc,
    clippy::collapsible_match,
    dead_code,
    unused_imports,
    clippy::len_zero
)]
#![no_std]
pub mod chardev;
pub mod cpu;
pub mod error;
pub mod icount;
pub mod irq;
pub mod memory;
pub mod net;
pub mod qdev;
pub mod qom;
pub mod ssi;
pub mod sync;
pub mod sysemu;
pub mod timer;

use core::ffi::c_char;

extern "C" {
    pub fn virtmcu_log(fmt: *const c_char);

    pub fn virtmcu_sizeof_device_state() -> usize;
    pub fn virtmcu_sizeof_sys_bus_device() -> usize;
    pub fn virtmcu_sizeof_device_class() -> usize;
    pub fn virtmcu_sizeof_ssi_peripheral() -> usize;
    pub fn virtmcu_sizeof_ssi_peripheral_class() -> usize;
    pub fn virtmcu_sizeof_chardev() -> usize;
    pub fn virtmcu_sizeof_chardev_class() -> usize;
    pub fn virtmcu_sizeof_char_backend() -> usize;
}

#[macro_export]
macro_rules! vlog {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut buf = [0u8; 256];
        let mut cursor = $crate::BufCursor::new(&mut buf);
        let _ = write!(cursor, $($arg)*);
        let _ = write!(cursor, "\0");
        unsafe { $crate::virtmcu_log(buf.as_ptr() as *const _) };
    }};
}

pub struct BufCursor<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl<'a> BufCursor<'a> {
    pub fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, pos: 0 }
    }
}

impl<'a> core::fmt::Write for BufCursor<'a> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let bytes = s.as_bytes();
        let len = bytes.len();
        if self.pos + len > self.buf.len() {
            return Err(core::fmt::Error);
        }
        self.buf[self.pos..self.pos + len].copy_from_slice(bytes);
        self.pos += len;
        Ok(())
    }
}
