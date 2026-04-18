use core::ffi::c_char;

#[repr(C)]
pub struct Error {
    _opaque: [u8; 0],
}

extern "C" {
    pub fn virtmcu_error_setg(errp: *mut *mut Error, fmt: *const c_char, ...);
}

#[macro_export]
macro_rules! error_setg {
    ($errp:expr, $fmt:expr) => {
        unsafe {
            $crate::error::virtmcu_error_setg($errp, $fmt);
        }
    };
}
