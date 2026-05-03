// std is required: virtmcu-qom dependency brings in std
//! Rust-dummy peripheral template for VirtMCU simulation.

use core::ffi::{c_uint, c_void};
use virtmcu_qom::memory::{
    memory_region_init_io, MemoryRegion, MemoryRegionOps, DEVICE_LITTLE_ENDIAN,
};
use virtmcu_qom::qdev::{sysbus_init_mmio, SysBusDevice};
use virtmcu_qom::qom::Property;
use virtmcu_qom::qom::{Object, ObjectClass, TypeInfo};
use virtmcu_qom::{declare_device_type, define_prop_uint64, device_class};

/// RustDummy peripheral structure
#[repr(C)]
pub struct RustDummyQEMU {
    /// Parent object
    pub parent_obj: SysBusDevice,
    /// I/O memory region
    pub iomem: MemoryRegion,
    /// Base address property
    pub base_addr: u64,
    /// Debug flag
    pub debug: bool,
}

unsafe extern "C" fn rust_dummy_read(_opaque: *mut c_void, addr: u64, _size: c_uint) -> u64 {
    let s = &*(_opaque as *mut RustDummyQEMU);

    match addr {
        0 => 0xdead_beef,
        8 => 0xface_babe,
        _ => {
            if s.debug {
                virtmcu_qom::sim_warn!("rust_dummy_read: unhandled offset 0x{:x}", addr);
            }
            0
        }
    }
}

unsafe extern "C" fn rust_dummy_write(_opaque: *mut c_void, addr: u64, val: u64, _size: c_uint) {
    let s = &*(_opaque as *mut RustDummyQEMU);
    if s.debug {
        virtmcu_qom::sim_warn!("rust_dummy_write: unhandled offset 0x{:x} val=0x{:x}", addr, val);
    }
}

static RUST_DUMMY_OPS: MemoryRegionOps = MemoryRegionOps {
    read: Some(rust_dummy_read),
    write: Some(rust_dummy_write),
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

unsafe extern "C" fn rust_dummy_realize(dev: *mut c_void, _errp: *mut *mut c_void) {
    let s = &mut *(dev as *mut RustDummyQEMU);

    memory_region_init_io(
        &raw mut s.iomem,
        dev as *mut Object,
        &raw const RUST_DUMMY_OPS,
        core::ptr::from_mut(s) as *mut c_void,
        c"rust-dummy".as_ptr(),
        0x1000,
    );
    sysbus_init_mmio(dev as *mut SysBusDevice, &raw mut s.iomem);
}

static RUST_DUMMY_PROPERTIES: [Property; 3] = [
    define_prop_uint64!(c"base-addr".as_ptr(), RustDummyQEMU, base_addr, u64::MAX),
    virtmcu_qom::define_prop_bool!(c"debug".as_ptr(), RustDummyQEMU, debug, false),
    // SAFETY: QEMU expects a zeroed Property as a sentinel at the end of the array.
    unsafe { core::mem::zeroed() },
];

unsafe extern "C" fn rust_dummy_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let dc = device_class!(klass);
    (*dc).realize = Some(rust_dummy_realize);
    (*dc).user_creatable = true;
    virtmcu_qom::qdev::device_class_set_props_n(dc, RUST_DUMMY_PROPERTIES.as_ptr(), 2);
}

#[used]
static RUST_DUMMY_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"rust-dummy".as_ptr(),
    parent: c"sys-bus-device".as_ptr(),
    instance_size: core::mem::size_of::<RustDummyQEMU>(),
    instance_align: 0,
    instance_init: None,
    instance_post_init: None,
    instance_finalize: None,
    abstract_: false,
    class_size: core::mem::size_of::<virtmcu_qom::qdev::SysBusDeviceClass>(),
    class_init: Some(rust_dummy_class_init),
    class_base_init: None,
    class_data: core::ptr::null(),
    interfaces: core::ptr::null(),
};

declare_device_type!(RUST_DUMMY_TYPE_INIT, RUST_DUMMY_TYPE_INFO);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rust_dummy_qemu_layout() {
        assert_eq!(
            core::mem::offset_of!(RustDummyQEMU, parent_obj),
            0,
            "SysBusDevice must be the first field"
        );
    }
}
