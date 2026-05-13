#![cfg_attr(
    test,
    allow(
        clippy::expect_used,
        clippy::unwrap_used,
        clippy::panic,
        clippy::indexing_slicing,
        clippy::panic_in_result_fn
    )
)]
// std is required: virtmcu-qom dependency brings in std
//! Rust-dummy peripheral template for VirtMCU simulation.

extern crate alloc;

use core::ffi::c_void;
use virtmcu_qom::memory::{memory_region_init_io, MemoryRegion};
use virtmcu_qom::qdev::{sysbus_init_mmio, SysBusDevice};
use virtmcu_qom::qom::Property;
use virtmcu_qom::qom::{Object, ObjectClass, TypeInfo};
use virtmcu_qom::{declare_device_type, define_prop_uint64, device_class};

const DUMMY_REG_8: u64 = 8;
const DUMMY_VAL_DEADBEEF: u64 = 0xdead_beef;
const DUMMY_VAL_FACEBABE: u64 = 0xface_babe;
const DUMMY_MMIO_SIZE: u64 = 0x1000;
const DUMMY_PROP_COUNT: usize = 2;

/// RustDummy peripheral structure
#[repr(C)]
#[derive(virtmcu_qom::MmioDevice)]
pub struct RustDummyQEMU {
    /// Parent object
    pub parent_obj: SysBusDevice,
    /// I/O memory region
    pub iomem: MemoryRegion,
    /// Base address property
    pub base_addr: u64,
    /// Debug flag
    pub debug: bool,

    /// Rust State
    pub rust_state: *mut RustDummyState,
}

pub struct RustDummyState {
    pub debug: bool,
    pub drain: virtmcu_qom::sync::VcpuDrain,
    pub cond: virtmcu_qom::sync::Condvar,
    pub wait_mutex: virtmcu_qom::sync::Mutex<()>,
}

impl virtmcu_qom::device::MmioDevice for RustDummyState {
    fn read(&self, addr: u64, _size: u32) -> virtmcu_qom::device::MmioResult<'_> {
        let _guard = self.drain.acquire();
        match addr {
            0 => virtmcu_qom::device::MmioResult::Ready(DUMMY_VAL_DEADBEEF),
            DUMMY_REG_8 => virtmcu_qom::device::MmioResult::Ready(DUMMY_VAL_FACEBABE),
            _ => {
                if self.debug {
                    virtmcu_qom::sim_warn!("rust_dummy_read: unhandled offset 0x{:x}", addr);
                }
                virtmcu_qom::device::MmioResult::Ready(0)
            }
        }
    }

    fn write(&self, addr: u64, val: u64, _size: u32) {
        let _guard = self.drain.acquire();
        if self.debug {
            virtmcu_qom::sim_warn!(
                "rust_dummy_write: unhandled offset 0x{:x} val=0x{:x}",
                addr,
                val
            );
        }
    }

    fn condvar(&self) -> &virtmcu_qom::sync::Condvar {
        &self.cond
    }

    fn wait_mutex(&self) -> &virtmcu_qom::sync::Mutex<()> {
        &self.wait_mutex
    }
}

unsafe extern "C" fn rust_dummy_realize(dev: *mut c_void, _errp: *mut *mut c_void) {
    let s = &mut *(dev as *mut RustDummyQEMU);

    let state = alloc::boxed::Box::new(RustDummyState {
        debug: s.debug,
        drain: virtmcu_qom::sync::VcpuDrain::new(),
        cond: virtmcu_qom::sync::Condvar::new(),
        wait_mutex: virtmcu_qom::sync::Mutex::new(()),
    });
    s.rust_state = alloc::boxed::Box::into_raw(state);

    memory_region_init_io(
        &raw mut s.iomem,
        dev as *mut Object,
        &raw const RUSTDUMMYQEMU_OPS,
        core::ptr::from_mut(s) as *mut c_void,
        c"rust-dummy".as_ptr(),
        DUMMY_MMIO_SIZE,
    );
    sysbus_init_mmio(dev as *mut SysBusDevice, &raw mut s.iomem);
}

unsafe extern "C" fn rust_dummy_instance_finalize(obj: *mut Object) {
    let s = unsafe { &mut *(obj as *mut RustDummyQEMU) };
    if !s.rust_state.is_null() {
        unsafe {
            drop(alloc::boxed::Box::from_raw(s.rust_state));
        }
        s.rust_state = core::ptr::null_mut();
    }
}

static RUST_DUMMY_PROPERTIES: [Property; DUMMY_PROP_COUNT] = [
    define_prop_uint64!(c"base-addr".as_ptr(), RustDummyQEMU, base_addr, u64::MAX),
    virtmcu_qom::define_prop_bool!(c"debug".as_ptr(), RustDummyQEMU, debug, false),
];

unsafe extern "C" fn rust_dummy_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let dc = device_class!(klass);
    (*dc).realize = Some(rust_dummy_realize);
    (*dc).user_creatable = true;
    virtmcu_qom::qdev::device_class_set_props_n(
        dc,
        RUST_DUMMY_PROPERTIES.as_ptr(),
        DUMMY_PROP_COUNT,
    );
}

#[used]
static RUST_DUMMY_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"rust-dummy".as_ptr(),
    parent: c"sys-bus-device".as_ptr(),
    instance_size: core::mem::size_of::<RustDummyQEMU>(),
    instance_align: 0,
    instance_init: None,
    instance_post_init: None,
    instance_finalize: Some(rust_dummy_instance_finalize),
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
