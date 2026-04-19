#![allow(
    clippy::missing_safety_doc,
    clippy::collapsible_match,
    dead_code,
    unused_imports,
    clippy::len_zero
)]
#![no_std]

use core::ffi::c_void;
use core::panic::PanicInfo;
use virtmcu_qom::qemu_log_mask;
use virtmcu_qom::qom::LOG_UNIMP;

#[no_mangle]
pub unsafe extern "C" fn rust_dummy_read(_priv_state: *mut c_void, addr: u64, _size: u32) -> u64 {
    qemu_log_mask!(LOG_UNIMP, "rust_dummy_read called from Rust!");

    match addr {
        0 => 0xdead_beef,
        8 => 0xface_babe,
        _ => 0,
    }
}

#[no_mangle]
pub unsafe extern "C" fn rust_dummy_write(
    _priv_state: *mut c_void,
    addr: u64,
    val: u64,
    _size: u32,
) {
    qemu_log_mask!(
        LOG_UNIMP,
        "rust_dummy_write called from Rust: addr=0x{:x}, val=0x{:x}",
        addr,
        val
    );
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
