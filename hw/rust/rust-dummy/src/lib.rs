#![no_std]

use core::ffi::c_void;
use core::panic::PanicInfo;
use virtmcu_qom::qom::{qemu_log_mask, LOG_UNIMP};

#[no_mangle]
pub extern "C" fn rust_dummy_read(_priv_state: *mut c_void, addr: u64, size: u32) -> u64 {
    let msg = b"rust_dummy_read called from Rust!\n\0";
    unsafe { qemu_log_mask(LOG_UNIMP, msg.as_ptr() as *const core::ffi::c_char) };

    match addr {
        0 => 0xdead_beef,
        _ => 0,
    }
}

#[no_mangle]
pub extern "C" fn rust_dummy_write(_priv_state: *mut c_void, _addr: u64, _val: u64, _size: u32) {
    let msg = b"rust_dummy_write called from Rust!\n\0";
    unsafe { qemu_log_mask(LOG_UNIMP, msg.as_ptr() as *const core::ffi::c_char) };
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    extern "C" { fn abort() -> !; }
    unsafe { abort() }
}
