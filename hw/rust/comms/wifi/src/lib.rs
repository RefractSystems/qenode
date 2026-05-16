#![allow(clippy::panic)] // virtmcu-allow: allow reasoning="Fail Loudly"
#![cfg_attr(
    test,
    allow(
        clippy::expect_used,
        clippy::unwrap_used,
        clippy::indexing_slicing,
        clippy::panic_in_result_fn
    )
)]
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

extern "C" fn wifi_read(_opaque: *mut c_void, addr: u64, _size: core::ffi::c_uint) -> u64 {
    let s = &*(_opaque as *mut VirtmcuWifiQEMU);
    if s.debug {
        virtmcu_qom::sim_debug!("wifi_read: unhandled offset 0x{:x}", addr);
    }
    0
}

extern "C" fn wifi_write(
    _opaque: *mut c_void,
    addr: u64,
    val: u64,
    _size: core::ffi::c_uint,
) {
    unreachable!("wifi_write: unhandled offset 0x{:x} val=0x{:x}", addr, val);
}

const WIFI_MAX_ACCESS: u32 = 8;
const WIFI_MMIO_SIZE: u64 = 0x1000;
const WIFI_PROPERTIES_COUNT: usize = 5;

static WIFI_OPS: MemoryRegionOps = MemoryRegionOps {
    read: Some(wifi_read),
    write: Some(wifi_write),
    read_with_attrs: core::ptr::null(),
    write_with_attrs: core::ptr::null(),
    endianness: DEVICE_LITTLE_ENDIAN,
    _padding1: [0; 4],
    valid: virtmcu_qom::memory::MemoryRegionValidRange {
        min_access_size: 1,
        max_access_size: WIFI_MAX_ACCESS,
        unaligned: false,
        _padding: [0; 7],
        accepts: core::ptr::null(),
    },
    impl_: virtmcu_qom::memory::MemoryRegionImplRange {
        min_access_size: 1,
        max_access_size: WIFI_MAX_ACCESS,
        unaligned: false,
        _padding: [0; 7],
    },
};


static WIFI_PROPERTIES: [Property; WIFI_PROPERTIES_COUNT] = [
    define_prop_macaddr!(c"macaddr".as_ptr(), VirtmcuWifiQEMU, mac),
    define_prop_string!(c"node".as_ptr(), VirtmcuWifiQEMU, node_id),
    define_prop_string!(c"transport".as_ptr(), VirtmcuWifiQEMU, transport),
    define_prop_string!(c"router".as_ptr(), VirtmcuWifiQEMU, router),
    virtmcu_qom::define_prop_bool!(c"debug".as_ptr(), VirtmcuWifiQEMU, debug, false),
];




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
