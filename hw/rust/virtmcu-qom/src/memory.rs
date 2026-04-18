use crate::qom::Object;
use core::ffi::{c_char, c_int, c_uint, c_void};

pub type MemoryRegionReadFn =
    unsafe extern "C" fn(opaque: *mut c_void, addr: u64, size: c_uint) -> u64;
pub type MemoryRegionWriteFn =
    unsafe extern "C" fn(opaque: *mut c_void, addr: u64, data: u64, size: c_uint);

#[repr(C)]
pub struct MemoryRegionOps {
    pub read: Option<MemoryRegionReadFn>,
    pub write: Option<MemoryRegionWriteFn>,
    pub read_with_attrs: *const c_void,
    pub write_with_attrs: *const c_void,
    pub endianness: c_int,
    pub _padding1: [u8; 4],
    pub valid: MemoryRegionValidRange,
    pub impl_: MemoryRegionImplRange,
}

#[repr(C)]
pub struct MemoryRegionValidRange {
    pub min_access_size: c_uint,
    pub max_access_size: c_uint,
    pub unaligned: bool,
    pub _padding: [u8; 7],
    pub accepts: *const c_void,
}

#[repr(C)]
pub struct MemoryRegionImplRange {
    pub min_access_size: c_uint,
    pub max_access_size: c_uint,
    pub unaligned: bool,
    pub _padding: [u8; 7],
}

#[repr(C, align(16))]
pub struct MemoryRegion {
    pub parent_obj: Object,
    pub _opaque: [u8; 272 - 40], // Pad to 272 bytes
}

pub const DEVICE_NATIVE_ENDIAN: c_int = 0;
pub const DEVICE_BIG_ENDIAN: c_int = 1;
pub const DEVICE_LITTLE_ENDIAN: c_int = 2;
pub const DEVICE_HOST_ENDIAN: c_int = 3;

extern "C" {
    pub fn memory_region_init_io(
        mr: *mut MemoryRegion,
        owner: *mut Object,
        ops: *const MemoryRegionOps,
        opaque: *mut c_void,
        name: *const c_char,
        size: u64,
    );
}

unsafe impl Sync for MemoryRegionOps {}
unsafe impl Sync for MemoryRegionValidRange {}
unsafe impl Sync for MemoryRegionImplRange {}
