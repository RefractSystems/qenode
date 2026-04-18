use core::ffi::{c_char, c_int, c_ulong, c_void};

pub const LOG_UNIMP: i32 = 0x400;

extern "C" {
    pub fn qemu_log_mask(mask: c_int, fmt: *const c_char, ...) -> c_int;
    pub fn type_register_static(info: *const TypeInfo) -> *mut c_void;
    pub fn device_class_set_props(dc: *mut DeviceClass, props: *const Property);
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
unsafe impl Sync for DeviceClass {}

#[repr(C)]
pub struct DeviceClass {
    pub parent_class: ObjectClass,
    pub categories: [c_ulong; 1],
    pub fw_name: *const c_char,
    pub desc: *const c_char,
    pub props_: *const Property,
    pub props_count_: u16,
    pub user_creatable: bool,
    pub hotpluggable: bool,
    pub legacy_reset: Option<unsafe extern "C" fn(dev: *mut c_void)>,
    pub realize: Option<unsafe extern "C" fn(dev: *mut c_void, errp: *mut *mut c_void)>,
    pub unrealize: Option<unsafe extern "C" fn(dev: *mut c_void)>,
    pub sync_config: Option<unsafe extern "C" fn(dev: *mut c_void, errp: *mut *mut c_void)>,
    pub vmsd: *const c_void,
    pub bus_type: *const c_char,
}

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
        pub static $name: [$crate::qom::Property; count_props!($($prop)*) + 1] = [
            $($prop,)*
            $crate::qom::Property::default(),
        ];
    };
}

#[macro_export]
macro_rules! count_props {
    () => (0);
    ($x:expr $(, $xs:expr)* $(,)?) => (1 + $crate::count_props!($($xs),*));
}
const _: () = assert!(core::mem::size_of::<TypeInfo>() == 104);
const _: () = assert!(core::mem::size_of::<Property>() == 72);

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
        pub static __DSO_INIT_PTR: extern "C" fn() = $init_fn;
    };
}
