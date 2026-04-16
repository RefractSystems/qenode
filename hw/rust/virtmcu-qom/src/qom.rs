use core::ffi::{c_char, c_int};

pub const LOG_UNIMP: i32 = 0x400; // Usually this is an enum, we just define the constant

extern "C" {
    /// Safe wrapper around qemu_log_mask.
    /// Note: `fmt` must be a null-terminated C string.
    pub fn qemu_log_mask(mask: c_int, fmt: *const c_char, ...) -> c_int;
}
