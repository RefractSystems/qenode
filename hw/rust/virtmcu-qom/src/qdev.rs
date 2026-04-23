use crate::qom::{Object, ObjectClass, Property};
use core::ffi::{c_char, c_int, c_void};

/// A constant
pub const TYPE_DEVICE: *const c_char = c"device".as_ptr();
/// A constant
pub const TYPE_SYS_BUS_DEVICE: *const c_char = c"sys-bus-device".as_ptr();
#[repr(C)]
/// A struct
pub struct DeviceState {
    /// A struct field
    pub parent_obj: Object,
    /// A struct field
    pub id: *mut c_char,
    /// A struct field
    pub canonical_path: *mut c_char,
    /// A struct field
    pub realized: bool,
    /// A struct field
    pub pending_deleted_event: bool,
    /// A struct field
    pub _opaque1: [u8; 6], // Padding to 64
    /// A struct field
    pub pending_deleted_expires_ms: i64,
    /// A struct field
    pub hotplugged: c_int,
    /// A struct field
    pub allow_unplug_during_migration: bool,
    /// A struct field
    pub _opaque2: [u8; 3], // Padding to 80
    /// A struct field
    pub parent_bus: *mut c_void,
    /// A struct field
    pub gpios: *mut c_void, // QLIST_HEAD
    /// A struct field
    pub clocks: *mut c_void, // QLIST_HEAD
    /// A struct field
    pub child_bus: *mut c_void, // QLIST_HEAD
    /// A struct field
    pub num_child_bus: c_int,
    /// A struct field
    pub instance_id_alias: c_int,
    /// A struct field
    pub alias_required_for_version: c_int,
    /// A struct field
    pub _opaque3: [u8; 28], // Remainder to 152
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
/// A struct
pub struct MACAddr {
    /// A struct field
    pub a: [u8; 6],
}

#[repr(C)]
/// A struct
pub struct SysBusDevice {
    /// A struct field
    pub parent_obj: DeviceState, // 152
    /// A struct field
    pub num_mmio: c_int, // 156
    /// A struct field
    pub _padding: [u8; 4], // 160 (Alignment)
    /// A struct field
    pub mmio: [SysBusMMIO; 32], // 160 + 32*16 = 160 + 512 = 672
    /// A struct field
    pub num_pio: c_int, // 676
    /// A struct field
    pub pio: [core::ffi::c_uint; 32], // 680 + 32*4 = 680 + 128 = 808
}

#[repr(C)]
#[derive(Copy, Clone)]
/// A struct
pub struct SysBusMMIO {
    /// A struct field
    pub addr: u64,
    /// A struct field
    pub memory: *mut crate::memory::MemoryRegion,
}

#[repr(C)]
/// A struct
pub struct DeviceClass {
    /// A struct field
    pub parent_class: ObjectClass, // 96
    /// A struct field
    pub categories: [core::ffi::c_ulong; 1],
    /// A struct field
    pub fw_name: *const c_char,
    /// A struct field
    pub desc: *const c_char,
    /// A struct field
    pub props_: *const Property,
    /// A struct field
    pub props_count_: u16,
    /// A struct field
    /// Whether the device is user-creatable
    pub user_creatable: bool,
    /// Whether the device is hotpluggable
    pub hotpluggable: bool,
    /// Padding for alignment
    pub _padding: [u8; 4],
    /// A legacy reset method
    pub legacy_reset: Option<unsafe extern "C" fn(dev: *mut c_void)>,
    /// Realize method
    pub realize: Option<unsafe extern "C" fn(dev: *mut c_void, errp: *mut *mut c_void)>,
    /// Unrealize method
    pub unrealize: Option<unsafe extern "C" fn(dev: *mut c_void)>,
    /// A struct field
    pub sync_config: Option<unsafe extern "C" fn(dev: *mut c_void, errp: *mut *mut c_void)>,
    /// A struct field
    pub vmsd: *const c_void,
    /// A struct field
    pub bus_type: *const c_char,
}

#[repr(C)]
/// A struct
pub struct SysBusDeviceClass {
    /// A struct field
    pub parent_class: DeviceClass,
    /// A struct field
    pub explicit_ofw_unit_address:
        Option<unsafe extern "C" fn(dev: *const SysBusDevice) -> *mut c_char>,
    /// A struct field
    pub connect_irq_notifier:
        Option<unsafe extern "C" fn(dev: *mut SysBusDevice, irq: crate::irq::qemu_irq)>,
}

#[repr(C)]
/// A struct
pub struct PropertyInfo {
    /// A struct field
    pub name: *const c_char,
    /// A struct field
    pub description: *const c_char,
    /// A struct field
    pub enum_table: *const c_void,
    /// A struct field
    pub print: Option<
        unsafe extern "C" fn(
            dev: *mut c_void,
            prop: *mut Property,
            f: *mut c_void,
            name: *const c_char,
        ),
    >,
    /// A struct field
    pub get_default_value: Option<unsafe extern "C" fn(prop: *mut Property, val: *mut u64)>,
    /// A struct field
    pub set_default_value: Option<unsafe extern "C" fn(prop: *mut Property, val: u64)>,
    /// A struct field
    pub set: Option<
        unsafe extern "C" fn(
            obj: *mut Object,
            visitor: *mut c_void,
            name: *const c_char,
            opaque: *mut c_void,
            errp: *mut *mut c_void,
        ),
    >,
    /// A struct field
    pub get: Option<
        unsafe extern "C" fn(
            obj: *mut Object,
            visitor: *mut c_void,
            name: *const c_char,
            opaque: *mut c_void,
            errp: *mut *mut c_void,
        ),
    >,
    /// A struct field
    pub release:
        Option<unsafe extern "C" fn(obj: *mut Object, name: *const c_char, opaque: *mut c_void)>,
}

unsafe impl Sync for PropertyInfo {}

#[cfg(not(miri))]
extern "C" {
    /// A static
    pub static qdev_prop_uint32: PropertyInfo;
    /// A static
    pub static qdev_prop_uint64: PropertyInfo;
    /// A static
    pub static qdev_prop_bool: PropertyInfo;
    /// A static
    pub static qdev_prop_string: PropertyInfo;
    /// A static
    pub static qdev_prop_macaddr: PropertyInfo;

    /// A function
    pub fn qdev_get_parent_bus(dev: *const DeviceState) -> *mut c_void;
    /// A function
    pub fn device_class_set_props_n(dc: *mut DeviceClass, props: *const Property, n: usize);
    /// A function
    pub fn sysbus_init_mmio(sbd: *mut SysBusDevice, mr: *mut crate::memory::MemoryRegion);
    /// A function
    pub fn sysbus_mmio_map(sbd: *mut SysBusDevice, n: c_int, addr: u64);
    /// A function
    pub fn sysbus_init_irq(sbd: *mut SysBusDevice, irq: *mut crate::irq::qemu_irq);
    /// A function
    pub fn sysbus_get_connected_irq(sbd: *mut SysBusDevice, n: c_int) -> crate::irq::qemu_irq;
}

#[cfg(miri)]
mod miri_statics {
    use super::*;
    const DUMMY_PROP: PropertyInfo = PropertyInfo {
        name: core::ptr::null(),
        description: core::ptr::null(),
        enum_table: core::ptr::null(),
        print: None,
        get_default_value: None,
        set_default_value: None,
        set: None,
        get: None,
        release: None,
    };
    /// A static
    #[no_mangle]
    pub static qdev_prop_uint32: PropertyInfo = DUMMY_PROP;
    /// A static
    #[no_mangle]
    pub static qdev_prop_uint64: PropertyInfo = DUMMY_PROP;
    /// A static
    #[no_mangle]
    pub static qdev_prop_bool: PropertyInfo = DUMMY_PROP;
    /// A static
    #[no_mangle]
    pub static qdev_prop_string: PropertyInfo = DUMMY_PROP;
    /// A static
    #[no_mangle]
    pub static qdev_prop_macaddr: PropertyInfo = DUMMY_PROP;

    extern "C" {
        /// A function
        pub fn qdev_get_parent_bus(dev: *const DeviceState) -> *mut c_void;
        /// A function
        pub fn device_class_set_props_n(dc: *mut DeviceClass, props: *const Property, n: usize);
        /// A function
        pub fn sysbus_init_mmio(sbd: *mut SysBusDevice, mr: *mut crate::memory::MemoryRegion);
        /// A function
        pub fn sysbus_mmio_map(sbd: *mut SysBusDevice, n: c_int, addr: u64);
        /// A function
        pub fn sysbus_init_irq(sbd: *mut SysBusDevice, irq: *mut crate::irq::qemu_irq);
        /// A function
        pub fn sysbus_get_connected_irq(sbd: *mut SysBusDevice, n: c_int) -> crate::irq::qemu_irq;
    }
}
#[cfg(miri)]
pub use miri_statics::*;

#[macro_export]
/// A macro
macro_rules! define_prop_macaddr {
    ($name:expr, $state:ty, $field:ident) => {
        $crate::qom::Property {
            name: $name,
            info: unsafe { &$crate::qdev::qdev_prop_macaddr as *const _ as *const _ },
            offset: core::mem::offset_of!($state, $field) as isize,
            link_type: core::ptr::null(),
            bitmask: 0,
            defval: 0,
            set_default: false,
            arrayinfo: core::ptr::null(),
            arrayfieldsize: 0,
            arrayoffset: 0,
            _padding: [0; 6],
            bitnr: 0,
        }
    };
}

#[macro_export]
/// A macro
macro_rules! device_class_set_props {
    ($dc:expr, $props:expr) => {
        unsafe {
            $crate::qdev::device_class_set_props_n($dc, $props.as_ptr(), $props.len());
        }
    };
}

#[macro_export]
/// A macro
macro_rules! define_properties {
    ($name:ident, [$($prop:expr),* $(,)?]) => {
        pub static $name: &[$crate::qom::Property] = &[
            $($prop),*
        ];
    };
}

#[macro_export]
/// A macro
macro_rules! define_prop_uint64 {
    ($name:expr, $state:ty, $field:ident, $default:expr) => {
        $crate::qom::Property {
            name: $name,
            info: unsafe { &$crate::qdev::qdev_prop_uint64 as *const _ as *const _ },
            offset: core::mem::offset_of!($state, $field) as isize,
            link_type: core::ptr::null(),
            bitmask: 0,
            defval: $default as u64,
            arrayinfo: core::ptr::null(),
            arrayoffset: 0,
            arrayfieldsize: 0,
            bitnr: 0,
            set_default: true,
            _padding: [0; 6],
        }
    };
}

#[macro_export]
/// A macro
macro_rules! define_prop_uint32 {
    ($name:expr, $state:ty, $field:ident, $default:expr) => {
        $crate::qom::Property {
            name: $name,
            info: unsafe { &$crate::qdev::qdev_prop_uint32 as *const _ as *const _ },
            offset: core::mem::offset_of!($state, $field) as isize,
            link_type: core::ptr::null(),
            bitmask: 0,
            defval: $default as u64,
            set_default: true,
            arrayinfo: core::ptr::null(),
            arrayoffset: 0,
            arrayfieldsize: 0,
            bitnr: 0,
            _padding: [0; 6],
        }
    };
}

#[macro_export]
/// A macro
macro_rules! define_prop_string {
    ($name:expr, $state:ty, $field:ident) => {
        $crate::qom::Property {
            name: $name,
            info: unsafe { &$crate::qdev::qdev_prop_string as *const _ as *const _ },
            offset: core::mem::offset_of!($state, $field) as isize,
            link_type: core::ptr::null(),
            bitmask: 0,
            defval: 0,
            set_default: false,
            arrayinfo: core::ptr::null(),
            arrayoffset: 0,
            arrayfieldsize: 0,
            bitnr: 0,
            _padding: [0; 6],
        }
    };
}

#[macro_export]
/// A macro
macro_rules! define_prop_chr {
    ($name:expr, $state:ty, $field:ident) => {
        $crate::qom::Property {
            name: $name,
            info: unsafe { &$crate::chardev::qdev_prop_chr as *const _ as *const _ },
            offset: core::mem::offset_of!($state, $field) as isize,
            link_type: core::ptr::null(),
            bitmask: 0,
            defval: 0,
            set_default: false,
            arrayinfo: core::ptr::null(),
            arrayoffset: 0,
            arrayfieldsize: 0,
            bitnr: 0,
            _padding: [0; 6],
        }
    };
}

const _: () = assert!(core::mem::size_of::<DeviceState>() == 152);
const _: () = assert!(core::mem::offset_of!(DeviceState, parent_obj) == 0);
const _: () = assert!(core::mem::size_of::<SysBusDevice>() == 808);
const _: () = assert!(core::mem::offset_of!(SysBusDevice, parent_obj) == 0);
const _: () = assert!(core::mem::size_of::<DeviceClass>() == 184);
const _: () = assert!(core::mem::offset_of!(DeviceClass, parent_class) == 0);
const _: () = assert!(core::mem::size_of::<SysBusDeviceClass>() == 200);
const _: () = assert!(core::mem::offset_of!(SysBusDeviceClass, parent_class) == 0);
