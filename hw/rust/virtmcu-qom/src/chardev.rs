use crate::qom::{Object, ObjectClass};
use core::ffi::{c_char, c_int, c_void};

#[repr(C)]
pub struct Chardev {
    pub parent_obj: Object,
    pub label: *mut c_char,
    pub filename: *mut c_char,
    pub log_append: bool,
    _padding: [u8; 7],
    pub log_chan: *mut c_void,    // QIOChannel *
    pub be: *mut c_void,          // CharBackend *
    pub gcontext: *mut c_void,    // GMainContext *
    pub chr_write_lock: [u8; 64], // QemuMutex
    _opaque: [u8; 160 - 40 - 8 - 8 - 1 - 7 - 8 - 8 - 8 - 64],
}

#[repr(C)]
pub struct ChardevClass {
    pub parent_class: ObjectClass, // 96
    pub internal: bool,            // 96
    _padding: [u8; 7],             // 97
    pub chr_parse: Option<
        unsafe extern "C" fn(opts: *mut c_void, backend: *mut c_void, errp: *mut *mut c_void),
    >, // 104
    pub chr_open: Option<
        unsafe extern "C" fn(
            chr: *mut Chardev,
            backend: *mut c_void,
            errp: *mut *mut c_void,
        ) -> bool,
    >, // 112
    pub chr_write:
        Option<unsafe extern "C" fn(chr: *mut Chardev, buf: *const u8, len: c_int) -> c_int>, // 120
    _opaque: [u8; 256 - 128],
}

extern "C" {
    pub fn qemu_chr_be_write(s: *mut Chardev, buf: *const u8, len: usize);
    pub fn qemu_chr_be_can_write(s: *mut Chardev) -> core::ffi::c_int;
}

const _: () = assert!(core::mem::size_of::<Chardev>() == 160);
const _: () = assert!(core::mem::size_of::<ChardevClass>() == 256);
const _: () = assert!(core::mem::offset_of!(ChardevClass, chr_write) == 120);
const _: () = assert!(core::mem::offset_of!(ChardevClass, chr_parse) == 104);
