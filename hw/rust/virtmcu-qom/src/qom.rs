use core::ffi::{c_char, c_int, c_ulong, c_void};

pub const LOG_UNIMP: i32 = 0x400;

extern "C" {
    pub fn qemu_log(fmt: *const c_char, ...);
    pub static qemu_loglevel: c_int;
    pub fn type_register_static(info: *const TypeInfo) -> *mut c_void;
    pub fn object_class_dynamic_cast_assert(
        klass: *mut ObjectClass,
        typename: *const c_char,
        file: *const c_char,
        line: c_int,
        func: *const c_char,
    ) -> *mut ObjectClass;
    pub fn object_class_get_name(klass: *mut ObjectClass) -> *const c_char;
    pub fn register_dso_module_init(fn_: unsafe extern "C" fn(), type_: c_int) -> c_void;
    pub fn object_get_canonical_path(obj: *mut Object) -> *mut c_char;
    pub fn object_get_root() -> *mut Object;
    pub fn object_dynamic_cast(obj: *mut Object, typename: *const c_char) -> *mut Object;
    pub fn object_child_foreach_recursive(
        obj: *mut Object,
        fn_: Option<unsafe extern "C" fn(obj: *mut Object, opaque: *mut c_void) -> c_int>,
        opaque: *mut c_void,
    ) -> c_int;
}

pub const TYPE_DEVICE: *const c_char = c"device".as_ptr();
pub const MODULE_INIT_QOM: c_int = 3;

#[macro_export]
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
pub struct Object {
    pub class: *mut ObjectClass,
    pub free: Option<unsafe extern "C" fn(obj: *mut Object)>,
    pub properties: *mut c_void,
    pub ref_: c_int,
    pub parent: *mut Object,
}

#[repr(C)]
pub struct ObjectClass {
    pub type_: *mut c_void,
    pub interfaces: *mut c_void,
    pub object_cast_cache: [*mut c_char; 4],
    pub class_cast_cache: [*mut c_char; 4],
    pub unparent: *mut c_void,
    pub properties: *mut c_void,
}

const _: () = assert!(core::mem::size_of::<ObjectClass>() == 96);

#[repr(C)]
pub struct TypeInfo {
    pub name: *const c_char,
    pub parent: *const c_char,
    pub instance_size: usize,
    pub instance_align: usize,
    pub instance_init: Option<unsafe extern "C" fn(obj: *mut Object)>,
    pub instance_post_init: Option<unsafe extern "C" fn(obj: *mut Object)>,
    pub instance_finalize: Option<unsafe extern "C" fn(obj: *mut Object)>,
    pub abstract_: bool,
    pub class_size: usize,
    pub class_init: Option<unsafe extern "C" fn(klass: *mut ObjectClass, data: *const c_void)>,
    pub class_base_init: Option<unsafe extern "C" fn(klass: *mut ObjectClass, data: *const c_void)>,
    pub class_data: *const c_void,
    pub interfaces: *const c_void,
}

#[repr(C)]
pub struct Property {
    pub name: *const c_char,
    pub info: *const c_void,
    pub offset: isize,
    pub link_type: *const c_char,
    pub bitmask: u64,
    pub defval: u64,
    pub arrayinfo: *const c_void,
    pub arrayoffset: c_int,
    pub arrayfieldsize: c_int,
    pub bitnr: u8,
    pub set_default: bool,
    pub _padding: [u8; 6],
}

const _: () = assert!(core::mem::size_of::<Property>() == 72);

unsafe impl Sync for TypeInfo {}
unsafe impl Sync for Property {}

#[macro_export]
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
