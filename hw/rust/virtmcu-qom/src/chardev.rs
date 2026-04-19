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
    pub be: *mut c_void,          // CharFrontend *
    pub gcontext: *mut c_void,    // GMainContext *
    pub chr_write_lock: [u8; 64], // QemuMutex
    _opaque: [u8; 160 - 40 - 8 - 8 - 1 - 7 - 8 - 8 - 8 - 64],
}

#[repr(C)]
#[derive(Default)]
pub struct CharFrontend {
    pub chr: *mut Chardev,
    pub chr_event: Option<unsafe extern "C" fn(opaque: *mut c_void, event: c_int)>,
    pub chr_can_read: Option<unsafe extern "C" fn(opaque: *mut c_void) -> c_int>,
    pub chr_read: Option<unsafe extern "C" fn(opaque: *mut c_void, buf: *const u8, size: c_int)>,
    pub chr_be_change: Option<unsafe extern "C" fn(opaque: *mut c_void) -> c_int>,
    pub opaque: *mut c_void,
    pub tag: core::ffi::c_uint,
    pub fe_is_open: bool,
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

    pub fn qemu_chr_fe_init(be: *mut CharFrontend, s: *mut Chardev, errp: *mut *mut c_void)
        -> bool;
    pub fn qemu_chr_fe_deinit(be: *mut CharFrontend, del: bool);
    pub fn qemu_chr_fe_set_handlers(
        be: *mut CharFrontend,
        fd_can_read: Option<unsafe extern "C" fn(opaque: *mut c_void) -> c_int>,
        fd_read: Option<unsafe extern "C" fn(opaque: *mut c_void, buf: *const u8, size: c_int)>,
        fd_event: Option<unsafe extern "C" fn(opaque: *mut c_void, event: c_int)>,
        be_change: Option<unsafe extern "C" fn(opaque: *mut c_void) -> c_int>,
        opaque: *mut c_void,
        context: *mut c_void,
        set_open: bool,
    );
    pub fn qemu_chr_fe_write(be: *mut CharFrontend, buf: *const u8, len: c_int) -> c_int;
    pub fn qemu_chr_fe_write_all(be: *mut CharFrontend, buf: *const u8, len: c_int) -> c_int;

    pub static qdev_prop_chr: crate::qdev::PropertyInfo;
}

const _: () = assert!(core::mem::size_of::<CharFrontend>() == 56);
const _: () = assert!(core::mem::size_of::<Chardev>() == 160);
const _: () = assert!(core::mem::size_of::<ChardevClass>() == 256);
const _: () = assert!(core::mem::offset_of!(ChardevClass, chr_write) == 120);
const _: () = assert!(core::mem::offset_of!(ChardevClass, chr_parse) == 104);
