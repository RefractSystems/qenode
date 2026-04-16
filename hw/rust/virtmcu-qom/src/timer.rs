use core::ffi::c_void;

pub const QEMU_CLOCK_VIRTUAL: i32 = 1;

#[repr(C)]
pub struct QemuTimer {
    _opaque: [u8; 0],
}

pub type QemuTimerCb = extern "C" fn(opaque: *mut c_void);

extern "C" {
    pub fn qemu_clock_get_ns(clock_type: i32) -> i64;
    pub fn timer_new_ns(clock_type: i32, cb: QemuTimerCb, opaque: *mut c_void) -> *mut QemuTimer;
    pub fn timer_mod(timer: *mut QemuTimer, expire_time: i64);
    pub fn timer_free(timer: *mut QemuTimer);
}
