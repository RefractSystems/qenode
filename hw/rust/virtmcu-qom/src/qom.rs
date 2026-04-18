use core::ffi::{c_char, c_int, c_ulong, c_void};

pub const LOG_UNIMP: i32 = 0x400;

extern "C" {
    pub fn qemu_log_mask(mask: c_int, fmt: *const c_char, ...) -> c_int;
    pub fn type_register_static(info: *const TypeInfo) -> *mut c_void;
    pub fn object_class_dynamic_cast_assert(
        klass: *mut ObjectClass,
        typename: *const c_char,
        file: *const c_char,
        line: c_int,
        func: *const c_char,
    ) -> *mut ObjectClass;
    pub fn register_dso_module_init(fn_: extern "C" fn(), type_: c_int);
}

pub const MODULE_INIT_QOM: c_int = 3;
pub const TYPE_DEVICE: *const c_char = c"device".as_ptr();

#[macro_export]
macro_rules! device_class {
    ($klass:expr) => {
        unsafe {
            $crate::qom::object_class_dynamic_cast_assert(
                $klass,
                $crate::qom::TYPE_DEVICE,
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
    pub free: *mut c_void,
    pub properties: *mut c_void,
    pub ref_: u32,
    pub parent: *mut Object,
}

#[repr(C)]
pub struct ObjectClass {
    pub type_: *mut c_void,
    pub interfaces: *mut c_void,
    pub object_cast_cache: [*const c_char; 4],
    pub class_cast_cache: [*const c_char; 4],
    pub unparent: Option<unsafe extern "C" fn(obj: *mut Object)>,
    pub properties: *mut c_void,
}

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
}

unsafe impl Sync for TypeInfo {}
unsafe impl Sync for Property {}

impl Property {
    pub const fn default() -> Self {
        Property {
            name: core::ptr::null(),
            info: core::ptr::null(),
            offset: 0,
            link_type: core::ptr::null(),
            bitmask: 0,
            defval: 0,
            arrayinfo: core::ptr::null(),
            arrayoffset: 0,
            arrayfieldsize: 0,
            bitnr: 0,
            set_default: false,
        }
    }
}

#[macro_export]
macro_rules! define_properties {
    ($name:ident, [ $($prop:expr),* $(,)? ]) => {
        pub static $name: [$crate::qom::Property; $crate::count_props!($($prop),*)] = [
            $($prop,)*
        ];
    };
}

#[macro_export]
macro_rules! count_props {
    ($($xs:expr),* $(,)?) => {
        0usize $(+ { let _ = stringify!($xs); 1usize })*
    };
}
const _: () = assert!(core::mem::size_of::<TypeInfo>() == 104);
const _: () = assert!(core::mem::size_of::<Property>() == 72);
const _: () = assert!(core::mem::size_of::<ObjectClass>() == 96);

#[macro_export]
macro_rules! declare_device_type {
    ($init_fn:ident, $type_info:ident) => {
        #[no_mangle]
        pub extern "C" fn $init_fn() {
            unsafe {
                $crate::qom::type_register_static(&$type_info);
            }
        }

        #[used]
        #[allow(non_upper_case_globals)]
        #[cfg_attr(target_os = "linux", link_section = ".init_array")]
        #[cfg_attr(target_os = "macos", link_section = "__DATA,__mod_init_func")]
        #[cfg_attr(target_os = "windows", link_section = ".CRT$XCU")]
        pub static __DSO_INIT_PTR: extern "C" fn() = {
            extern "C" fn wrapper() {
                unsafe {
                    $crate::qom::register_dso_module_init($init_fn, $crate::qom::MODULE_INIT_QOM);
                }
            }
            wrapper
        };
    };
}
