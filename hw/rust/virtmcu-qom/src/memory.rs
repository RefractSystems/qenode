use crate::qom::Object;
use core::ffi::{c_char, c_int, c_uint, c_void};

/// A type alias
pub type MemoryRegionReadFn =
    unsafe extern "C" fn(opaque: *mut c_void, addr: u64, size: c_uint) -> u64;
/// A type alias
pub type MemoryRegionWriteFn =
    unsafe extern "C" fn(opaque: *mut c_void, addr: u64, data: u64, size: c_uint);

#[repr(C)]
/// A struct
pub struct MemoryRegionOps {
    /// A struct field
    pub read: Option<MemoryRegionReadFn>,
    /// A struct field
    pub write: Option<MemoryRegionWriteFn>,
    /// A struct field
    pub read_with_attrs: *const c_void,
    /// A struct field
    pub write_with_attrs: *const c_void,
    /// A struct field
    pub endianness: c_int,
    /// A struct field
    pub _padding1: [u8; 4],
    /// A struct field
    pub valid: MemoryRegionValidRange,
    /// A struct field
    pub impl_: MemoryRegionImplRange,
}

#[repr(C)]
/// A struct
pub struct MemoryRegionValidRange {
    /// A struct field
    pub min_access_size: c_uint,
    /// A struct field
    pub max_access_size: c_uint,
    /// A struct field
    pub unaligned: bool,
    /// A struct field
    pub _padding: [u8; 7],
    /// A struct field
    pub accepts: *const c_void,
}

#[repr(C)]
/// A struct
pub struct MemoryRegionImplRange {
    /// A struct field
    pub min_access_size: c_uint,
    /// A struct field
    pub max_access_size: c_uint,
    /// A struct field
    pub unaligned: bool,
    /// A struct field
    pub _padding: [u8; 7],
}

#[repr(C, align(16))]
/// A struct
pub struct MemoryRegion {
    /// A struct field
    pub parent_obj: Object,
    /// A struct field
    pub _opaque: [u8; 272 - 40], // Pad to 272 bytes
}

/// A constant
pub const DEVICE_NATIVE_ENDIAN: c_int = 0;
/// A constant
pub const DEVICE_BIG_ENDIAN: c_int = 1;
/// A constant
pub const DEVICE_LITTLE_ENDIAN: c_int = 2;
/// A constant
pub const DEVICE_HOST_ENDIAN: c_int = 3;

extern "C" {
    /// A function
    pub fn memory_region_init_io(
        mr: *mut MemoryRegion,
        owner: *mut Object,
        ops: *const MemoryRegionOps,
        opaque: *mut c_void,
        name: *const c_char,
        size: u64,
    );
}

// SAFETY: These structs are passed to QEMU which expects them to be constant
// and accessible from multiple threads (e.g. vCPU threads). Since they only
// contain function pointers and static configuration, they are safe to be Sync.
unsafe impl Sync for MemoryRegionOps {}
// SAFETY: Static configuration for memory regions.
unsafe impl Sync for MemoryRegionValidRange {}
// SAFETY: Static configuration for memory regions.
unsafe impl Sync for MemoryRegionImplRange {}
