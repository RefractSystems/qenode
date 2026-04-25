use core::ffi::{c_char, c_int, c_void};

/// A constant
pub const LOG_UNIMP: i32 = 0x400;

extern "C" {
    /// A function
    pub fn qemu_log(fmt: *const c_char, ...);
    /// A static
    pub static qemu_loglevel: c_int;
    /// A function
    pub fn type_register_static(info: *const TypeInfo) -> *mut c_void;
    /// A function
    pub fn object_class_dynamic_cast_assert(
        klass: *mut ObjectClass,
        typename: *const c_char,
        file: *const c_char,
        line: c_int,
        func: *const c_char,
    ) -> *mut ObjectClass;
    /// A function
    pub fn object_class_get_name(klass: *mut ObjectClass) -> *const c_char;
    /// A function
    pub fn register_dso_module_init(fn_: unsafe extern "C" fn(), type_: c_int);
    /// A function
    pub fn object_get_canonical_path(obj: *mut Object) -> *mut c_char;
    /// A function
    pub fn object_get_root() -> *mut Object;
    /// A function
    pub fn object_dynamic_cast(obj: *mut Object, typename: *const c_char) -> *mut Object;
    /// A function
    pub fn object_child_foreach_recursive(
        obj: *mut Object,
        fn_: Option<unsafe extern "C" fn(obj: *mut Object, opaque: *mut c_void) -> c_int>,
        opaque: *mut c_void,
    ) -> c_int;
}

/// A constant
pub const TYPE_DEVICE: *const c_char = c"device".as_ptr();
/// A constant
pub const MODULE_INIT_QOM: c_int = 3;

#[macro_export]
/// A macro
macro_rules! qemu_log_mask {
    ($mask:expr, $($arg:tt)*) => {{
        unsafe {
            if ($crate::qom::qemu_loglevel & $mask) != 0 {
                $crate::vlog!($($arg)*);
            }
        }
    }};
}

#[macro_export]
/// A macro
macro_rules! device_class {
    ($klass:expr) => {
        unsafe {
            $crate::qom::object_class_dynamic_cast_assert(
                $klass,
                $crate::qdev::TYPE_DEVICE,
                core::ptr::null(),
                0,
                core::ptr::null(),
            ) as *mut $crate::qdev::DeviceClass
        }
    };
}

#[repr(C)]
/// A struct
pub struct Object {
    /// A struct field
    pub class: *mut ObjectClass,
    /// A struct field
    pub free: Option<unsafe extern "C" fn(obj: *mut Object)>,
    /// A struct field
    pub properties: *mut c_void,
    /// A struct field
    pub ref_: c_int,
    /// A struct field
    pub parent: *mut Object,
}

#[repr(C)]
/// A struct
pub struct ObjectClass {
    /// A struct field
    pub type_: *mut c_void,
    /// A struct field
    pub interfaces: *mut c_void,
    /// A struct field
    pub object_cast_cache: [*mut c_char; 4],
    /// A struct field
    pub class_cast_cache: [*mut c_char; 4],
    /// A struct field
    pub unparent: *mut c_void,
    /// A struct field
    pub properties: *mut c_void,
}

const _: () = assert!(core::mem::size_of::<ObjectClass>() == 96);

#[repr(C)]
/// A struct
pub struct TypeInfo {
    /// A struct field
    pub name: *const c_char,
    /// A struct field
    pub parent: *const c_char,
    /// A struct field
    pub instance_size: usize,
    /// A struct field
    pub instance_align: usize,
    /// A struct field
    pub instance_init: Option<unsafe extern "C" fn(obj: *mut Object)>,
    /// A struct field
    pub instance_post_init: Option<unsafe extern "C" fn(obj: *mut Object)>,
    /// A struct field
    pub instance_finalize: Option<unsafe extern "C" fn(obj: *mut Object)>,
    /// A struct field
    pub abstract_: bool,
    /// A struct field
    pub class_size: usize,
    /// A struct field
    pub class_init: Option<unsafe extern "C" fn(klass: *mut ObjectClass, data: *const c_void)>,
    /// A struct field
    pub class_base_init: Option<unsafe extern "C" fn(klass: *mut ObjectClass, data: *const c_void)>,
    /// A struct field
    pub class_data: *const c_void,
    /// A struct field
    pub interfaces: *const c_void,
}

#[repr(C)]
/// A struct
pub struct Property {
    /// A struct field
    pub name: *const c_char,
    /// A struct field
    pub info: *const c_void,
    /// A struct field
    pub offset: isize,
    /// A struct field
    pub link_type: *const c_char,
    /// A struct field
    pub bitmask: u64,
    /// A struct field
    pub defval: u64,
    /// A struct field
    pub arrayinfo: *const c_void,
    /// A struct field
    pub arrayoffset: c_int,
    /// A struct field
    pub arrayfieldsize: c_int,
    /// A struct field
    pub bitnr: u8,
    /// A struct field
    pub set_default: bool,
    /// A struct field
    pub _padding: [u8; 6],
}

const _: () = assert!(core::mem::size_of::<Property>() == 72);

unsafe impl Sync for TypeInfo {}
unsafe impl Sync for Property {}
unsafe impl Send for Property {}

#[macro_export]
/// A macro
macro_rules! declare_device_type {
    ($init_fn:ident, $type_info:expr) => {
        #[used]
        #[no_mangle]
        #[allow(non_upper_case_globals)]
        #[cfg_attr(target_os = "linux", link_section = ".init_array")]
        #[cfg_attr(target_os = "macos", link_section = "__DATA,__mod_init_func")]
        #[cfg_attr(target_os = "windows", link_section = ".CRT$XCU")]
        pub static $init_fn: extern "C" fn() = {
            extern "C" fn wrapper() {
                #[cfg(not(miri))]
                unsafe {
                    $crate::qom::register_dso_module_init(real_init, $crate::qom::MODULE_INIT_QOM);
                }
            }
            unsafe extern "C" fn real_init() {
                $crate::qom::type_register_static(&$type_info);
            }
            wrapper
        };
    };
}
