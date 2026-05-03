// std is required: virtmcu-qom dependency brings in std
#![allow(missing_docs)]
#![allow(clippy::missing_safety_doc)]

use core::ffi::{c_char, c_void};
use core::ptr;
use virtmcu_qom::memory::{
    memory_region_init_io, MemoryRegion, MemoryRegionOps, DEVICE_LITTLE_ENDIAN,
};
use virtmcu_qom::qdev::{sysbus_init_mmio, MACAddr, SysBusDevice};
use virtmcu_qom::qom::{Object, ObjectClass, Property, TypeInfo};
use virtmcu_qom::{declare_device_type, define_prop_macaddr, define_prop_string, device_class};

#[repr(C)]
pub struct VirtmcuWifiQEMU {
    pub parent_obj: SysBusDevice,
    pub mmio: MemoryRegion,
    pub mac: MACAddr,
    pub node_id: *mut c_char,
    pub transport: *mut c_char,
    pub router: *mut c_char,
    pub debug: bool,
}

unsafe extern "C" fn wifi_read(_opaque: *mut c_void, addr: u64, _size: core::ffi::c_uint) -> u64 {
    let s = &*(_opaque as *mut VirtmcuWifiQEMU);
    if s.debug {
        virtmcu_qom::sim_warn!("wifi_read: unhandled offset 0x{:x}", addr);
    }
    0
}

unsafe extern "C" fn wifi_write(
    _opaque: *mut c_void,
    addr: u64,
    val: u64,
    _size: core::ffi::c_uint,
) {
    let s = &*(_opaque as *mut VirtmcuWifiQEMU);
    if s.debug {
        virtmcu_qom::sim_warn!("wifi_write: unhandled offset 0x{:x} val=0x{:x}", addr, val);
    }
}

static WIFI_OPS: MemoryRegionOps = MemoryRegionOps {
    read: Some(wifi_read),
    write: Some(wifi_write),
    read_with_attrs: core::ptr::null(),
    write_with_attrs: core::ptr::null(),
    endianness: DEVICE_LITTLE_ENDIAN,
    _padding1: [0; 4],
    valid: virtmcu_qom::memory::MemoryRegionValidRange {
        min_access_size: 1,
        max_access_size: 8,
        unaligned: false,
        _padding: [0; 7],
        accepts: core::ptr::null(),
    },
    impl_: virtmcu_qom::memory::MemoryRegionImplRange {
        min_access_size: 1,
        max_access_size: 8,
        unaligned: false,
        _padding: [0; 7],
    },
};

unsafe extern "C" fn wifi_realize(dev: *mut c_void, errp: *mut *mut c_void) {
    let s = &mut *(dev as *mut VirtmcuWifiQEMU);

    let is_zero = s.mac.a.iter().all(|&x| x == 0);
    if is_zero {
        virtmcu_qom::error_setg!(
            errp,
            "wifi: macaddr property must be set to a non-zero address\n"
        );
        return;
    }

    memory_region_init_io(
        &raw mut s.mmio,
        dev as *mut Object,
        &raw const WIFI_OPS,
        core::ptr::from_mut(s) as *mut c_void,
        c"wifi-mmio".as_ptr(),
        0x1000,
    );
    sysbus_init_mmio(dev as *mut SysBusDevice, &raw mut s.mmio);
}

static WIFI_PROPERTIES: [Property; 6] = [
    define_prop_macaddr!(c"macaddr".as_ptr(), VirtmcuWifiQEMU, mac),
    define_prop_string!(c"node".as_ptr(), VirtmcuWifiQEMU, node_id),
    define_prop_string!(c"transport".as_ptr(), VirtmcuWifiQEMU, transport),
    define_prop_string!(c"router".as_ptr(), VirtmcuWifiQEMU, router),
    virtmcu_qom::define_prop_bool!(c"debug".as_ptr(), VirtmcuWifiQEMU, debug, false),
    // SAFETY: QEMU expects a zeroed Property as a sentinel at the end of the array.
    unsafe { core::mem::zeroed() },
];

unsafe extern "C" fn wifi_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let dc = device_class!(klass);
    (*dc).realize = Some(wifi_realize);
    (*dc).user_creatable = true;
    virtmcu_qom::qdev::device_class_set_props_n(dc, WIFI_PROPERTIES.as_ptr(), 5);
}

#[used]
static WIFI_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"wifi".as_ptr(),
    parent: virtmcu_qom::qdev::TYPE_SYS_BUS_DEVICE,
    instance_size: core::mem::size_of::<VirtmcuWifiQEMU>(),
    instance_align: 0,
    instance_init: None,
    instance_post_init: None,
    instance_finalize: None,
    abstract_: false,
    class_size: core::mem::size_of::<virtmcu_qom::qdev::SysBusDeviceClass>(),
    class_init: Some(wifi_class_init),
    class_base_init: None,
    class_data: core::ptr::null(),
    interfaces: ptr::null(),
};

declare_device_type!(VIRTMCU_WIFI_TYPE_INIT, WIFI_TYPE_INFO);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wifi_qemu_layout() {
        assert_eq!(
            core::mem::offset_of!(VirtmcuWifiQEMU, parent_obj),
            0,
            "SysBusDevice must be the first field"
        );
    }
}
