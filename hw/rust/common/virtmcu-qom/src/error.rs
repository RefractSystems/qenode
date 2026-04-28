use core::ffi::c_char;

#[repr(C)]
/// A struct
pub struct Error {
    _opaque: [u8; 0],
}

extern "C" {
    /// A function
    pub fn virtmcu_error_setg(errp: *mut *mut Error, fmt: *const c_char);
}

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
