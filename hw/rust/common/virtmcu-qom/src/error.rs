use core::ffi::c_char;

#[repr(C)]
/// A struct
pub struct Error {
    _opaque: [u8; 0],
}

#[cfg(not(any(test, miri, feature = "standalone", virtmcu_unit_test)))]
extern "C" {
    /// A function
    pub fn virtmcu_error_setg(errp: *mut *mut Error, fmt: *const c_char);
    /// A function
    pub fn error_free(err: *mut Error);
}

#[cfg(any(test, miri, feature = "standalone", virtmcu_unit_test))]
#[no_mangle]
/// Stub for virtmcu_error_setg in tests and standalone mode.
pub unsafe extern "C" fn virtmcu_error_setg(_errp: *mut *mut Error, _fmt: *const c_char) {}

#[cfg(any(test, miri, feature = "standalone", virtmcu_unit_test))]
#[no_mangle]
/// Stub for error_free in tests and standalone mode.
pub unsafe extern "C" fn error_free(_err: *mut Error) {}

#[macro_export]
/// Sets a generic QEMU error with a formatted message.
macro_rules! error_setg {
    ($errp:expr, $($arg:tt)*) => {{
        use core::fmt::Write;
        let mut buf = [0u8; 1024];
        let mut cursor = $crate::BufCursor::new(&mut buf);
        let _ = write!(cursor, $($arg)*);
        let _ = write!(cursor, "\0");
        // SAFETY: virtmcu_error_setg takes a null-terminated string. buf contains a
        // null-terminated string. The buffer is alive for the duration of the call.
        unsafe {
            $crate::error::virtmcu_error_setg(
                $errp as *mut *mut $crate::error::Error,
                buf.as_ptr() as *const _,
            );
        }
    }};
}
