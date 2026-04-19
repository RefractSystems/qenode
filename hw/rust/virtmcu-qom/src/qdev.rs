use crate::qom::{Object, ObjectClass, Property};
use core::ffi::{c_char, c_int, c_void};

pub const TYPE_DEVICE: *const c_char = c"device".as_ptr();
pub const TYPE_SYS_BUS_DEVICE: *const c_char = c"sys-bus-device".as_ptr();

#[repr(C)]
pub struct DeviceState {
    pub parent_obj: Object,
    pub id: *mut c_char,
    pub canonical_path: *mut c_char,
    pub realized: bool,
    pub pending_deleted_event: bool,
    pub _opaque1: [u8; 6], // Padding to 64
    pub pending_deleted_expires_ms: i64,
    pub hotplugged: c_int,
    pub allow_unplug_during_migration: bool,
    pub _opaque2: [u8; 3], // Padding to 80
    pub parent_bus: *mut c_void,
    pub gpios: *mut c_void,
    pub clocks: *mut c_void,
    pub child_bus: *mut c_void,
    pub num_child_bus: c_int,
    pub instance_id_alias: c_int,
    pub _opaque3: [u8; 32], // Remainder to 152
}

#[repr(C)]
pub struct SysBusDevice {
    pub parent_obj: DeviceState,      // 152
    pub num_mmio: c_int,              // 156
    pub _padding: [u8; 4],            // 160 (Alignment)
    pub mmio: [SysBusMMIO; 32],       // 160 + 32*16 = 160 + 512 = 672
    pub num_pio: c_int,               // 676
    pub pio: [core::ffi::c_uint; 32], // 680 + 32*4 = 680 + 128 = 808
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct SysBusMMIO {
    pub addr: u64,
    pub memory: *mut crate::memory::MemoryRegion,
}

#[repr(C)]
pub struct DeviceClass {
    pub parent_class: ObjectClass, // 96
    pub categories: [core::ffi::c_ulong; 1],
    pub fw_name: *const c_char,
    pub desc: *const c_char,
    pub props_: *const Property,
    pub props_count_: u16,
    pub user_creatable: bool,
    pub hotpluggable: bool,
    pub _padding: [u8; 4],
    pub legacy_reset: Option<unsafe extern "C" fn(dev: *mut c_void)>,
    pub realize: Option<unsafe extern "C" fn(dev: *mut c_void, errp: *mut *mut c_void)>,
    pub unrealize: Option<unsafe extern "C" fn(dev: *mut c_void)>,
    pub sync_config: Option<unsafe extern "C" fn(dev: *mut c_void, errp: *mut *mut c_void)>,
    pub vmsd: *const c_void,
    pub bus_type: *const c_char,
}

#[repr(C)]
pub struct SysBusDeviceClass {
    pub parent_class: DeviceClass,
    pub explicit_ofw_unit_address:
        Option<unsafe extern "C" fn(dev: *const SysBusDevice) -> *mut c_char>,
    pub connect_irq_notifier:
        Option<unsafe extern "C" fn(dev: *mut SysBusDevice, irq: crate::irq::qemu_irq)>,
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
    pub set: Option<
        unsafe extern "C" fn(
            obj: *mut Object,
            visitor: *mut c_void,
            name: *const c_char,
            opaque: *mut c_void,
            errp: *mut *mut c_void,
        ),
    >,
    pub get: Option<
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
    pub static qdev_prop_uint64: PropertyInfo;
    pub static qdev_prop_bool: PropertyInfo;
    pub static qdev_prop_string: PropertyInfo;

    pub fn device_class_set_props_n(dc: *mut DeviceClass, props: *const Property, n: usize);
    pub fn sysbus_init_mmio(sbd: *mut SysBusDevice, mr: *mut crate::memory::MemoryRegion);
    pub fn sysbus_init_irq(sbd: *mut SysBusDevice, irq: *mut crate::irq::qemu_irq);
    pub fn sysbus_get_connected_irq(sbd: *mut SysBusDevice, n: c_int) -> crate::irq::qemu_irq;
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
macro_rules! define_properties {
    ($name:ident, [$($prop:expr),* $(,)?]) => {
        pub static $name: &[$crate::qom::Property] = &[
            $($prop),*
        ];
    };
}

#[macro_export]
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
const _: () = assert!(core::mem::size_of::<SysBusDevice>() == 808);
const _: () = assert!(core::mem::size_of::<DeviceClass>() == 184);
const _: () = assert!(core::mem::size_of::<SysBusDeviceClass>() == 200);
