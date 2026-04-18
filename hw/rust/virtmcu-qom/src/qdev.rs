use crate::qom::{Object, ObjectClass, Property};
use core::ffi::{c_char, c_int, c_void};

#[repr(C)]
pub struct DeviceState {
    pub parent_obj: Object,
    pub id: *mut c_char,
    pub canonical_path: *mut c_char,
    pub realized: bool,
    _opaque: [u8; 152 - 57], // Pad to 152 bytes
}

#[repr(C)]
pub struct SysBusDevice {
    pub parent_obj: DeviceState,
    _opaque: [u8; 808 - 152], // Pad to 808 bytes
}

#[repr(C)]
pub struct DeviceClass {
    pub parent_class: ObjectClass,
    pub categories: [core::ffi::c_ulong; 1],
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

#[repr(C)]
pub struct PropertyInfo {
    pub name: *const c_char,
    pub description: *const c_char,
    pub enum_table: *const c_void,
    pub print: Option<
        unsafe extern "C" fn(
            dev: *mut c_void,
            prop: *mut Property,
            f: *mut c_void,
            name: *const c_char,
        ),
    >,
    pub get_default_value: Option<unsafe extern "C" fn(prop: *mut Property, val: *mut u64)>,
    pub set_default_value: Option<unsafe extern "C" fn(prop: *mut Property, val: u64)>,
    pub get: Option<
        unsafe extern "C" fn(
            obj: *mut Object,
            visitor: *mut c_void,
            name: *const c_char,
            opaque: *mut c_void,
            errp: *mut *mut c_void,
        ),
    >,
    pub set: Option<
        unsafe extern "C" fn(
            obj: *mut Object,
            visitor: *mut c_void,
            name: *const c_char,
            opaque: *mut c_void,
            errp: *mut *mut c_void,
        ),
    >,
    pub release:
        Option<unsafe extern "C" fn(obj: *mut Object, name: *const c_char, opaque: *mut c_void)>,
}

extern "C" {
    pub static qdev_prop_uint32: PropertyInfo;
    pub static qdev_prop_string: PropertyInfo;
    pub fn device_class_set_props_n(dc: *mut DeviceClass, props: *const Property, n: usize);
}

#[macro_export]
macro_rules! device_class_set_props {
    ($dc:expr, $props:expr) => {
        unsafe {
            $crate::qdev::device_class_set_props_n($dc, $props.as_ptr(), $props.len());
        }
    };
}

#[macro_export]
macro_rules! define_prop_uint32 {
    ($name:expr, $state:ty, $field:ident, $default:expr) => {
        $crate::qom::Property {
            name: $name,
            info: unsafe { &$crate::qdev::qdev_prop_uint32 as *const _ as *const _ },
            offset: core::mem::offset_of!($state, $field) as isize,
            bitmask: 0,
            defval: $default as u64,
            set_default: true,
            ..$crate::qom::Property::default()
        }
    };
}

#[macro_export]
macro_rules! define_prop_string {
    ($name:expr, $state:ty, $field:ident) => {
        $crate::qom::Property {
            name: $name,
            info: unsafe { &$crate::qdev::qdev_prop_string as *const _ as *const _ },
            offset: core::mem::offset_of!($state, $field) as isize,
            bitmask: 0,
            defval: 0,
            set_default: false,
            ..$crate::qom::Property::default()
        }
    };
}

const _: () = assert!(core::mem::size_of::<DeviceState>() == 152);
const _: () = assert!(core::mem::size_of::<SysBusDevice>() == 808);
const _: () = assert!(core::mem::size_of::<DeviceClass>() == 184);
