#![feature(prelude_import)]
#![allow(clippy::panic)]
// virtmcu-allow: allow reasoning="Fail Loudly"
#![allow(clippy::if_not_else)]
// std is required: virtmcu-qom dependency brings in std
//! ============================================================================
//! Welcome to the VirtMCU Peripheral Template!
//!
//! This file is the "Gold Standard" implementation mandated by ADR-021.
//! ============================================================================
extern crate std;
#[prelude_import]
use std::prelude::rust_2021::*;

extern crate alloc;

use alloc::sync::Arc;
use core::ffi::c_char;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use virtmcu_api::DataTransport;
use virtmcu_qom::memory::MemoryRegion;
use virtmcu_qom::qdev::SysBusDevice;
use virtmcu_qom::sync::{
    DeterministicReceiver, DeliveryPacket, Condvar, Mutex, VcpuDrain,
};

const DUMMY_REG_STATUS: u64 = 0x00;
const DUMMY_REG_TX: u64 = 0x04;
const DUMMY_REG_8: u64 = 0x08;
const DUMMY_VAL_FACEBABE: u64 = 0xface_babe;

/// A custom packet structure representing incoming data.
pub struct DummyPacket {
    pub vtime: u64,
    pub data: alloc::vec::Vec<u8>,
}
#[automatically_derived]
impl ::core::cmp::Eq for DummyPacket {
    #[inline]
    #[doc(hidden)]
    #[coverage(off)]
    fn assert_fields_are_eq(&self) {
        let _: ::core::cmp::AssertParamIsEq<u64>;
        let _: ::core::cmp::AssertParamIsEq<alloc::vec::Vec<u8>>;
    }
}
#[automatically_derived]
impl ::core::marker::StructuralPartialEq for DummyPacket { }
#[automatically_derived]
impl ::core::cmp::PartialEq for DummyPacket {
    #[inline]
    fn eq(&self, other: &DummyPacket) -> bool {
        self.vtime == other.vtime && self.data == other.data
    }
}

impl PartialOrd for DummyPacket {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DummyPacket {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.vtime.cmp(&other.vtime)
    }
}

impl DeliveryPacket for DummyPacket {
    fn delivery_vtime_ns(&self) -> u64 { self.vtime }
}

#[doc = " The QEMU C-FFI Boundary Object."]
#[doc = " (RFC-0023 Phase 4: Zero Unsafe Boilerplate)"]
#[repr(C)]
pub struct RustDummyQEMU {
    pub parent_obj: SysBusDevice,
    pub iomem: MemoryRegion,

    pub base_addr: u64,
    pub debug: bool,
    pub node_id: u32,
    pub topic: *mut c_char,

    pub transport: virtmcu_qom::qom::QomLink<dyn DataTransport>,

    pub state: *mut RustDummyState,
}
#[used]
static RUSTDUMMYQEMU_PROPERTIES: [virtmcu_qom::qom::Property; 4usize] =
    [




            // Initialize Deterministic Ingress (RFC-0023 Step 1 & 4)
            // Placeholder for tracer bullet verification









            // RFC-0023 Phase 5: DSO Registration

            ::virtmcu_qom::qom::Property {
                name: b"base-addr\0".as_ptr() as *const core::ffi::c_char,
                info: unsafe {
                    &::virtmcu_qom::qdev::qdev_prop_uint64 as *const _ as
                        *const _
                },
                offset: const {
                            builtin # offset_of(RustDummyQEMU, base_addr)
                        } as isize,
                link_type: core::ptr::null(),
                bitmask: 0,
                defval: 0 as u64,
                arrayinfo: core::ptr::null(),
                arrayoffset: 0,
                arrayfieldsize: 0,
                bitnr: 0,
                set_default: true,
                _padding: [0; 6],
            },
            ::virtmcu_qom::qom::Property {
                name: b"debug\0".as_ptr() as *const core::ffi::c_char,
                info: unsafe {
                    &::virtmcu_qom::qdev::qdev_prop_bool as *const _ as *const _
                },
                offset: const { builtin # offset_of(RustDummyQEMU, debug) } as
                    isize,
                link_type: core::ptr::null(),
                bitmask: 0,
                defval: 0 as u64,
                set_default: true,
                arrayinfo: core::ptr::null(),
                arrayoffset: 0,
                arrayfieldsize: 0,
                bitnr: 0,
                _padding: [0; 6],
            },
            ::virtmcu_qom::qom::Property {
                name: b"node-id\0".as_ptr() as *const core::ffi::c_char,
                info: unsafe {
                    &::virtmcu_qom::qdev::qdev_prop_uint32 as *const _ as
                        *const _
                },
                offset: const { builtin # offset_of(RustDummyQEMU, node_id) }
                    as isize,
                link_type: core::ptr::null(),
                bitmask: 0,
                defval: 0 as u64,
                set_default: true,
                arrayinfo: core::ptr::null(),
                arrayoffset: 0,
                arrayfieldsize: 0,
                bitnr: 0,
                _padding: [0; 6],
            },
            ::virtmcu_qom::qom::Property {
                name: b"topic\0".as_ptr() as *const core::ffi::c_char,
                info: unsafe {
                    &::virtmcu_qom::qdev::qdev_prop_string as *const _ as
                        *const _
                },
                offset: const { builtin # offset_of(RustDummyQEMU, topic) } as
                    isize,
                link_type: core::ptr::null(),
                bitmask: 0,
                defval: 0,
                set_default: false,
                arrayinfo: core::ptr::null(),
                arrayoffset: 0,
                arrayfieldsize: 0,
                bitnr: 0,
                _padding: [0; 6],
            }];
unsafe extern "C" fn allow_set_link(_obj: *mut virtmcu_qom::qom::Object,
    _name: *const core::ffi::c_char, _val: *mut virtmcu_qom::qom::Object,
    _errp: *mut *mut virtmcu_qom::error::Error) {}
unsafe extern "C" fn rustdummyqemu_instance_init(obj:
        *mut virtmcu_qom::qom::Object) {
    let s = unsafe { &mut *(obj as *mut RustDummyQEMU) };
    s.state = core::ptr::null_mut();
}
unsafe extern "C" fn rustdummyqemu_finalize(obj:
        *mut virtmcu_qom::qom::Object) {
    let s = unsafe { &mut *(obj as *mut RustDummyQEMU) };
    if !s.state.is_null() {
        unsafe { drop(Box::from_raw(s.state)); }
        s.state = core::ptr::null_mut();
    }
}
unsafe extern "C" fn rustdummyqemu_realize(dev: *mut core::ffi::c_void,
    _errp: *mut *mut core::ffi::c_void) {
    let s = unsafe { &mut *(dev as *mut RustDummyQEMU) };
    if !s.state.is_null() { return; }
    let mut state =
        Box::new(<RustDummyState as
                    virtmcu_qom::device::PeripheralState>::new(s));
    if let Err(e) = virtmcu_qom::device::Peripheral::realize(&mut *state) {
        {
            ::virtmcu_qom::telemetry::sim_log(::virtmcu_qom::telemetry::LogLevel::Error,
                "rust_dummy",
                format_args!("{0}: realization failed: {1}", "rust-dummy",
                    e));
        };
    }
    s.state = Box::into_raw(state);
    virtmcu_qom::memory::memory_region_init_io(&raw mut s.iomem,
        dev as *mut virtmcu_qom::qom::Object, &raw const RUSTDUMMYQEMU_OPS,
        core::ptr::from_mut(s) as *mut core::ffi::c_void,
        b"rust-dummy\0".as_ptr() as *const core::ffi::c_char, 0x1000);
    virtmcu_qom::qdev::sysbus_init_mmio(dev as
            *mut virtmcu_qom::qdev::SysBusDevice, &raw mut s.iomem);
}
unsafe extern "C" fn rustdummyqemu_class_init(klass:
        *mut virtmcu_qom::qom::ObjectClass, _data: *const core::ffi::c_void) {
    let dc =
        unsafe {
            ::virtmcu_qom::qom::object_class_dynamic_cast_assert(klass,
                    ::virtmcu_qom::qdev::TYPE_DEVICE, core::ptr::null(), 0,
                    core::ptr::null()) as *mut ::virtmcu_qom::qdev::DeviceClass
        };
    (*dc).realize = Some(rustdummyqemu_realize);
    (*dc).user_creatable = true;
    virtmcu_qom::qdev::device_class_set_props_n(dc,
        RUSTDUMMYQEMU_PROPERTIES.as_ptr(), 4usize);
    unsafe {
        virtmcu_qom::qom::object_class_property_add_link(klass,
            b"transport\0".as_ptr() as *const core::ffi::c_char,
            b"virtmcu-transport-hub\0".as_ptr() as *const core::ffi::c_char,
            const { builtin # offset_of(RustDummyQEMU, transport) } as isize,
            Some(allow_set_link), virtmcu_qom::qom::OBJ_PROP_LINK_STRONG);
    }
}
#[used]
pub static RUSTDUMMYQEMU_TYPE_INFO: virtmcu_qom::qom::TypeInfo =
    virtmcu_qom::qom::TypeInfo {
        name: b"rust-dummy\0".as_ptr() as *const core::ffi::c_char,
        parent: b"sys-bus-device\0".as_ptr() as *const core::ffi::c_char,
        instance_size: core::mem::size_of::<RustDummyQEMU>(),
        instance_align: 0,
        instance_init: Some(rustdummyqemu_instance_init),
        instance_post_init: None,
        instance_finalize: Some(rustdummyqemu_finalize),
        abstract_: false,
        class_size: core::mem::size_of::<virtmcu_qom::qdev::SysBusDeviceClass>(),
        class_init: Some(rustdummyqemu_class_init),
        class_base_init: None,
        class_data: core::ptr::null(),
        interfaces: core::ptr::null(),
    };
const BQL_YIELD_TIMEOUT_MS: u32 = 100;
const MAX_ACCESS_SIZE: u32 = 8;
unsafe extern "C" fn rustdummyqemu_read_shim(opaque: *mut core::ffi::c_void,
    offset: u64, size: core::ffi::c_uint) -> u64 {
    let s = unsafe { &mut *(opaque as *mut RustDummyQEMU) };
    let state_ptr = s.state;
    if state_ptr.is_null() { return 0; }
    let state = unsafe { &*state_ptr };
    let mut res =
        virtmcu_qom::device::MmioDevice::read(state, offset, size as u32);
    match res {
        virtmcu_qom::device::MmioResult::Ready(val) => val,
        virtmcu_qom::device::MmioResult::Wait {
            mut condition, mut ready_val, mut fallback_val } => {
            if condition() {
                ready_val()
            } else {
                {
                    let _unlock = virtmcu_qom::sync::Bql::temporary_unlock();
                    std::thread::yield_now();
                }
                fallback_val()
            }
        }
    }
}
unsafe extern "C" fn rustdummyqemu_write_shim(opaque: *mut core::ffi::c_void,
    offset: u64, value: u64, size: core::ffi::c_uint) {
    let s = unsafe { &mut *(opaque as *mut RustDummyQEMU) };
    let state_ptr = s.state;
    if state_ptr.is_null() { return; }
    let state = unsafe { &*state_ptr };
    virtmcu_qom::device::MmioDevice::write(state, offset, value, size as u32);
}
pub static RUSTDUMMYQEMU_OPS: virtmcu_qom::memory::MemoryRegionOps =
    virtmcu_qom::memory::MemoryRegionOps {
        read: Some(rustdummyqemu_read_shim),
        write: Some(rustdummyqemu_write_shim),
        read_with_attrs: core::ptr::null(),
        write_with_attrs: core::ptr::null(),
        endianness: virtmcu_qom::memory::DEVICE_LITTLE_ENDIAN,
        _padding1: [0; 4],
        valid: virtmcu_qom::memory::MemoryRegionValidRange {
            min_access_size: 1,
            max_access_size: MAX_ACCESS_SIZE,
            unaligned: false,
            _padding: [0; 7],
            accepts: core::ptr::null(),
        },
        impl_: virtmcu_qom::memory::MemoryRegionImplRange {
            min_access_size: 1,
            max_access_size: MAX_ACCESS_SIZE,
            unaligned: false,
            _padding: [0; 7],
        },
    };
pub struct RustDummyState {
    pub debug: bool,
    pub node_id: u32,
    pub drain: VcpuDrain,
    pub cond: Condvar,
    pub wait_mutex: Mutex<()>,
    pub transport: Option<Arc<dyn DataTransport>>,
    pub receiver: Option<DeterministicReceiver<DummyPacket>>,
    pub generation: Arc<AtomicU64>,
    pub has_data: AtomicBool,
}
impl virtmcu_qom::device::PeripheralState for RustDummyState {
    type QomType = RustDummyQEMU;
    fn new(qemu_dev: &Self::QomType) -> Self {
        Self {
            debug: qemu_dev.debug,
            node_id: qemu_dev.node_id,
            drain: VcpuDrain::new(),
            cond: Condvar::new(),
            wait_mutex: Mutex::new(()),
            transport: qemu_dev.transport.get(),
            receiver: None,
            generation: Arc::new(AtomicU64::new(0)),
            has_data: AtomicBool::new(false),
        }
    }
}
impl virtmcu_qom::device::Peripheral for RustDummyState {
    fn realize(&mut self) -> Result<(), alloc::string::String> {
        if self.transport.is_some() {}
        Ok(())
    }
    fn read(&self, addr: u64, size: u32,
        _token: &virtmcu_qom::device::DrainToken)
        -> virtmcu_qom::device::MmioResult<'_> {
        virtmcu_qom::device::MmioDevice::read(self, addr, size)
    }
    fn write(&self, addr: u64, val: u64, size: u32,
        _token: &virtmcu_qom::device::DrainToken) {
        virtmcu_qom::device::MmioDevice::write(self, addr, val, size);
    }
    fn condvar(&self) -> &Condvar { &self.cond }
    fn wait_mutex(&self) -> &Mutex<()> { &self.wait_mutex }
}
impl virtmcu_qom::device::MmioDevice for RustDummyState {
    fn read(&self, addr: u64, _size: u32)
        -> virtmcu_qom::device::MmioResult<'_> {
        let _guard = self.drain.acquire();
        match addr {
            DUMMY_REG_STATUS => {
                virtmcu_qom::device::MmioResult::wait_for(||
                        self.has_data.load(Ordering::Acquire), || 1, || 0)
            }
            DUMMY_REG_8 =>
                virtmcu_qom::device::MmioResult::Ready(DUMMY_VAL_FACEBABE),
            _ => virtmcu_qom::device::MmioResult::Ready(0),
        }
    }
    fn write(&self, addr: u64, val: u64, _size: u32) {
        let _guard = self.drain.acquire();
        match addr {
            DUMMY_REG_TX => {
                if let Some(transport) = &self.transport {
                    let _ =
                        transport.publish("sim/dummy/tx", &val.to_le_bytes());
                }
            }
            _ => {}
        }
    }
    fn condvar(&self) -> &Condvar { &self.cond }
    fn wait_mutex(&self) -> &Mutex<()> { &self.wait_mutex }
}
#[used]
#[no_mangle]
#[link_section = ".init_array"]
pub static RUSTDUMMYQEMU_INIT: extern "C" fn() =
    {
        extern "C" fn wrapper() {
            unsafe {
                if ::core::option::Option::None::<&'static str>.is_none() {
                    ::virtmcu_qom::qom::register_dso_module_init(real_init,
                        ::virtmcu_qom::qom::MODULE_INIT_QOM);
                }
            }
        }
        unsafe extern "C" fn real_init() {
            ::virtmcu_qom::qom::type_register_static(&RUSTDUMMYQEMU_TYPE_INFO);
        }
        wrapper
    };
