use crate::qom::{Object, ObjectClass};
use core::ffi::{c_char, c_int, c_void};

#[repr(C)]
/// A struct
pub struct Chardev {
    /// A struct field
    pub parent_obj: Object,
    /// A struct field
    pub chr_write_lock: [u8; 64], // QemuMutex at offset 40
    /// A struct field
    pub fe: *mut c_void, // CharFrontend * at offset 104
    /// A struct field
    pub label: *mut c_char, // at offset 112
    /// A struct field
    pub logfd: c_int, // at offset 120
    /// A struct field
    pub logtimestamp: bool, // at offset 124
    /// A struct field
    pub log_line_start: bool, // at offset 125
    _pad0: [u8; 2],
    /// A struct field
    pub be_open: c_int, // at offset 128
    /// A struct field
    pub handover_yank_instance: bool, // at offset 132
    _pad1: [u8; 3],
    /// A struct field
    pub gsource: *mut c_void, // at offset 136
    /// A struct field
    pub gcontext: *mut c_void, // at offset 144
    /// A struct field
    pub features: [u8; 8], // DECLARE_BITMAP at offset 152
}

#[repr(C)]
#[derive(Default)]
/// A struct
pub struct CharFrontend {
    /// A struct field
    pub chr: *mut Chardev,
    /// A struct field
    pub chr_event: Option<unsafe extern "C" fn(opaque: *mut c_void, event: c_int)>,
    /// A struct field
    pub chr_can_read: Option<unsafe extern "C" fn(opaque: *mut c_void) -> c_int>,
    /// A struct field
    pub chr_read: Option<unsafe extern "C" fn(opaque: *mut c_void, buf: *const u8, size: c_int)>,
    /// A struct field
    pub chr_be_change: Option<unsafe extern "C" fn(opaque: *mut c_void) -> c_int>,
    /// A struct field
    pub opaque: *mut c_void,
    /// A struct field
    pub tag: core::ffi::c_uint,
    /// A struct field
    pub fe_is_open: bool,
}

#[repr(C)]
/// A struct
pub struct ChardevClass {
    /// A struct field
    pub parent_class: ObjectClass, // 0
    /// A struct field
    pub internal: bool, // 96
    /// A struct field
    pub supports_yank: bool, // 97
    _padding: [u8; 6], // to offset 104
    /// A struct field
    pub chr_parse: Option<
        unsafe extern "C" fn(opts: *mut c_void, backend: *mut c_void, errp: *mut *mut c_void),
    >, // 104
    /// A struct field
    pub chr_open: Option<
        unsafe extern "C" fn(
            chr: *mut Chardev,
            backend: *mut c_void,
            errp: *mut *mut c_void,
        ) -> bool,
    >, // 112
    /// A struct field
    pub chr_write:
        Option<unsafe extern "C" fn(chr: *mut Chardev, buf: *const u8, len: c_int) -> c_int>, // 120
    _opaque: [u8; 256 - 128],
}

extern "C" {
    /// A function
    pub fn qemu_chr_be_write(s: *mut Chardev, buf: *const u8, len: usize);
    /// A function
    pub fn qemu_chr_be_can_write(s: *mut Chardev) -> core::ffi::c_int;

    /// A function
    pub fn qemu_chr_fe_init(be: *mut CharFrontend, s: *mut Chardev, errp: *mut *mut c_void)
        -> bool;
    /// A function
    pub fn qemu_chr_fe_deinit(be: *mut CharFrontend, del: bool);
    /// A function
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
    /// A function
    pub fn qemu_chr_fe_write(be: *mut CharFrontend, buf: *const u8, len: c_int) -> c_int;
    /// A function
    pub fn qemu_chr_fe_write_all(be: *mut CharFrontend, buf: *const u8, len: c_int) -> c_int;

    /// A static
    pub static qdev_prop_chr: crate::qdev::PropertyInfo;
}

const _: () = assert!(core::mem::size_of::<CharFrontend>() == 56);
const _: () = assert!(core::mem::size_of::<Chardev>() == 160);
const _: () = assert!(core::mem::size_of::<ChardevClass>() == 256);
const _: () = assert!(core::mem::offset_of!(ChardevClass, chr_write) == 120);
const _: () = assert!(core::mem::offset_of!(ChardevClass, chr_parse) == 104);
const _: () = assert!(core::mem::offset_of!(Chardev, chr_write_lock) == 40);
const _: () = assert!(core::mem::offset_of!(Chardev, label) == 112);
