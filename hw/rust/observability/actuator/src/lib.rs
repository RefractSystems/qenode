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
unsafe extern "C" fn allow_set_link(
    _obj: *mut virtmcu_qom::qom::Object,
    _name: *const core::ffi::c_char,
    _val: *mut virtmcu_qom::qom::Object,
    _errp: *mut *mut virtmcu_qom::error::Error,
) {
}
// Virtmcu actuator device with pluggable transport.

extern crate alloc;

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ffi::{c_char, c_void};
use core::ptr;
use virtmcu_api::topics::sim_topic;
use virtmcu_qom::memory::{memory_region_init_io, MemoryRegion};
use virtmcu_qom::qdev::{sysbus_init_mmio, SysBusDevice};
use virtmcu_qom::qom::{Object, ObjectClass, TypeInfo};
use virtmcu_qom::sync::VcpuDrain;
use virtmcu_qom::{
    declare_device_type, define_prop_string, define_prop_uint32, define_properties, device_class,
    error_setg,
};

#[repr(C)]
#[derive(virtmcu_qom::MmioDevice)]
pub struct VirtmcuActuatorQEMU {
    pub parent_obj: SysBusDevice,
    pub mmio: MemoryRegion,

    /* Properties */
    pub node_id: u32,
    pub transport: *mut c_char,
    pub router: *mut c_char,
    pub topic_prefix: *mut c_char,
    pub debug: bool,

    /* Links */
    pub transport_hub: *mut Object,

    /* Registers */
    pub actuator_id: u32,
    pub data_size: u32,
    pub data: [f64; 8],

    /* Rust state */
    pub rust_state: *mut VirtmcuActuatorState,
}

const MAX_DATA_ELEMENTS: usize = 8;
const F64_SIZE_BYTES: u64 = core::mem::size_of::<f64>() as u64;

const REG_ACTUATOR_ID: u64 = 0x00;
const REG_ACTUATOR_DATA_SIZE: u64 = 0x04;
const REG_ACTUATOR_GO: u64 = 0x08;
const REG_ACTUATOR_DATA: u64 = 0x10;

pub struct VirtmcuActuatorState {
    pub qemu_dev_ptr: *mut VirtmcuActuatorQEMU,
    pub drain: VcpuDrain,
    pub transport: Arc<dyn virtmcu_api::DataTransport>,
    pub seq: core::sync::atomic::AtomicU64,
    pub cond: virtmcu_qom::sync::Condvar,
    // virtmcu-allow: mutex reasoning="Required for Condvar::wait_yielding_bql"
    pub wait_mutex: virtmcu_qom::sync::Mutex<()>,
    pub _liveliness: Option<alloc::boxed::Box<dyn virtmcu_api::LivelinessToken>>,
}

impl virtmcu_qom::device::MmioDevice for VirtmcuActuatorState {
    fn read(&self, addr: u64, size: u32) -> virtmcu_qom::device::MmioResult<'_> {
        let _guard = self.drain.acquire();
        let s = unsafe { &mut *self.qemu_dev_ptr };
        let ret = match addr {
            REG_ACTUATOR_ID => u64::from(s.actuator_id),
            REG_ACTUATOR_DATA_SIZE => u64::from(s.data_size),
            addr if (REG_ACTUATOR_DATA
                ..REG_ACTUATOR_DATA + (MAX_DATA_ELEMENTS as u64) * F64_SIZE_BYTES)
                .contains(&addr) =>
            {
                let idx = ((addr - REG_ACTUATOR_DATA) / F64_SIZE_BYTES) as usize;
                let offset = ((addr - REG_ACTUATOR_DATA) % F64_SIZE_BYTES) as usize;
                let mut ret: u64 = 0;
                if offset + (size as usize) <= (F64_SIZE_BYTES as usize) {
                    let bytes = s.data.get(idx).expect("idx out of bounds").to_le_bytes();
                    let mut ret_bytes = [0u8; core::mem::size_of::<f64>()];
                    if let (Some(dest), Some(src)) = (
                        ret_bytes.get_mut(..size as usize),
                        bytes.get(offset..offset + size as usize),
                    ) {
                        dest.copy_from_slice(src);
                        ret = u64::from_le_bytes(ret_bytes);
                    }
                }
                ret
            }
            _ => {
                if s.debug {
                    virtmcu_qom::sim_debug!("actuator_read: unhandled offset 0x{:x}", addr);
                }
                0
            }
        };
        virtmcu_qom::device::MmioResult::Ready(ret)
    }

    fn write(&self, addr: u64, val: u64, size: u32) {
        let _guard = self.drain.acquire();
        let s = unsafe { &mut *self.qemu_dev_ptr };
        if s.debug {
            virtmcu_qom::vlog!("actuator_write: addr 0x{:x}, val {}\n", addr, val);
        }
        match addr {
            REG_ACTUATOR_ID => {
                s.actuator_id = val as u32;
            }
            REG_ACTUATOR_DATA_SIZE => {
                s.data_size = val as u32;
                if s.data_size > (MAX_DATA_ELEMENTS as u32) {
                    s.data_size = MAX_DATA_ELEMENTS as u32;
                }
            }
            REG_ACTUATOR_GO => {
                if (val & 0x1) == 1 {
                    let vtime_ns = u64::try_from(unsafe {
                        virtmcu_qom::timer::qemu_clock_get_ns(
                            virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL,
                        )
                    })
                    .expect("vtime is negative");

                    let seq = self.seq.fetch_add(1, core::sync::atomic::Ordering::Relaxed);

                    let node_id_str = s.node_id.to_string();
                    let topic = sim_topic::actuator_control(&node_id_str, s.actuator_id);
                    let mut data_payload =
                        Vec::with_capacity((s.data_size as usize) * (F64_SIZE_BYTES as usize));
                    for val in s.data.iter().take(s.data_size as usize) {
                        data_payload.extend_from_slice(&val.to_le_bytes());
                    }
                    let payload = virtmcu_api::encode_frame(vtime_ns, seq, &data_payload);

                    match self.transport.as_ref().reserve(&topic, payload.len()) {
                        Ok(mut reservation) => {
                            reservation.buffer_mut().copy_from_slice(&payload);
                            let _ = reservation.commit(vtime_ns, seq);
                        }
                        Err(e) => {
                            virtmcu_qom::sim_err!(
                                "actuator: failed to reserve transport for topic {topic}: {e:?}",
                            );
                        }
                    };
                }
            }
            addr if (REG_ACTUATOR_DATA
                ..REG_ACTUATOR_DATA + (MAX_DATA_ELEMENTS as u64) * F64_SIZE_BYTES)
                .contains(&addr) =>
            {
                let idx = ((addr - REG_ACTUATOR_DATA) / F64_SIZE_BYTES) as usize;
                let offset = ((addr - REG_ACTUATOR_DATA) % F64_SIZE_BYTES) as usize;
                if offset + (size as usize) <= (F64_SIZE_BYTES as usize) {
                    let val_bytes = val.to_le_bytes();
                    let mut data_bytes = s.data.get(idx).expect("idx out of bounds").to_le_bytes();
                    if let (Some(dest), Some(src)) = (
                        data_bytes.get_mut(offset..offset + size as usize),
                        val_bytes.get(..size as usize),
                    ) {
                        dest.copy_from_slice(src);
                        *s.data.get_mut(idx).expect("idx out of bounds") =
                            f64::from_le_bytes(data_bytes);
                    }
                }
            }
            _ => {
                if s.debug {
                    virtmcu_qom::sim_debug!(
                        "actuator_write: unhandled offset 0x{:x} val=0x{:x}",
                        addr,
                        val
                    );
                }
            }
        }
    }

    fn condvar(&self) -> &virtmcu_qom::sync::Condvar {
        &self.cond
    }

    // virtmcu-allow: mutex reasoning="Required for Condvar::wait_yielding_bql"
    fn wait_mutex(&self) -> &virtmcu_qom::sync::Mutex<()> {
        &self.wait_mutex
    }
}

/// # Safety
/// This function is called by QEMU to realize the device. dev must be a valid pointer to VirtmcuActuatorQEMU.
#[no_mangle]
pub unsafe extern "C" fn actuator_realize(dev: *mut c_void, errp: *mut *mut c_void) {
    const ACTUATOR_MMIO_SIZE: u64 = 0x1000;
    virtmcu_qom::vlog!("ACTUATOR_REALIZE CALLED\n");
    let s = unsafe { &mut *(dev as *mut VirtmcuActuatorQEMU) };

    if !s.rust_state.is_null() {
        return;
    }

    unsafe {
        memory_region_init_io(
            &raw mut s.mmio,
            dev as *mut Object,
            &raw const VIRTMCUACTUATORQEMU_OPS,
            dev,
            c"actuator".as_ptr(),
            ACTUATOR_MMIO_SIZE,
        );
        sysbus_init_mmio(dev as *mut SysBusDevice, &raw mut s.mmio);
    }

    if s.transport_hub.is_null() {
        error_setg!(errp, "Strict DI violation: actuator transport_hub link is required.");
        return;
    }

    unsafe {
        virtmcu_qom::qom::object_property_set_bool(
            s.transport_hub,
            c"realized".as_ptr(),
            true,
            errp as *mut *mut virtmcu_qom::error::Error,
        );
    }
    let ptr_u64 = unsafe {
        virtmcu_qom::qom::object_property_get_uint(
            s.transport_hub,
            c"transport_ptr".as_ptr(),
            errp as *mut *mut virtmcu_qom::error::Error,
        )
    };
    if ptr_u64 == 0 {
        virtmcu_qom::error_setg!(
            errp,
            "Strict DI violation: failed to acquire transport from hub."
        );
        return;
    }
    let transport_ref =
        unsafe { &*(ptr_u64 as *const alloc::sync::Arc<dyn virtmcu_api::DataTransport>) };
    let transport_arc = alloc::sync::Arc::clone(transport_ref);

    s.rust_state = actuator_init_internal(s, s.node_id, transport_arc);
    if s.rust_state.is_null() {
        error_setg!(errp, "actuator: failed to initialize Rust backend");
    }
}

/// # Safety
/// This function is called by QEMU when finalizing the device. obj must be a valid pointer to VirtmcuActuatorQEMU.
#[no_mangle]
pub unsafe extern "C" fn actuator_instance_finalize(obj: *mut Object) {
    let s = unsafe { &mut *(obj as *mut VirtmcuActuatorQEMU) };
    if !s.rust_state.is_null() {
        unsafe {
            drop(Box::from_raw(s.rust_state));
        }
        s.rust_state = ptr::null_mut();
    }
}

/// # Safety
/// This function is called by QEMU on instance initialization. obj must be a valid pointer to VirtmcuActuatorQEMU.
#[no_mangle]
pub unsafe extern "C" fn actuator_instance_init(obj: *mut Object) {
    let s = unsafe { &mut *(obj as *mut VirtmcuActuatorQEMU) };
    s.topic_prefix = ptr::null_mut();
    s.transport = ptr::null_mut();
}

define_properties!(
    VIRTMCU_ACTUATOR_PROPERTIES,
    [
        define_prop_uint32!(c"node".as_ptr(), VirtmcuActuatorQEMU, node_id, 0),
        define_prop_string!(c"router".as_ptr(), VirtmcuActuatorQEMU, router),
        define_prop_string!(c"topic-prefix".as_ptr(), VirtmcuActuatorQEMU, topic_prefix),
        virtmcu_qom::define_prop_bool!(c"debug".as_ptr(), VirtmcuActuatorQEMU, debug, false),
    ]
);

/// # Safety
/// This function is called by QEMU to initialize the class. klass must be a valid pointer to ObjectClass.
#[no_mangle]
pub unsafe extern "C" fn actuator_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let dc = device_class!(klass);
    unsafe {
        (*dc).realize = Some(actuator_realize);
        (*dc).user_creatable = true;
    }
    virtmcu_qom::device_class_set_props!(dc, VIRTMCU_ACTUATOR_PROPERTIES);

    unsafe {
        virtmcu_qom::qom::object_class_property_add_link(
            klass,
            c"transport".as_ptr(),
            c"virtmcu-transport-hub".as_ptr(),
            core::mem::offset_of!(VirtmcuActuatorQEMU, transport_hub) as isize,
            Some(allow_set_link),
            virtmcu_qom::qom::OBJ_PROP_LINK_STRONG,
        );
    }
}

#[used]
static VIRTMCU_ACTUATOR_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"actuator".as_ptr(),
    parent: c"sys-bus-device".as_ptr(),
    instance_size: core::mem::size_of::<VirtmcuActuatorQEMU>(),
    instance_align: 0,
    instance_init: Some(actuator_instance_init),
    instance_post_init: None,
    instance_finalize: Some(actuator_instance_finalize),
    abstract_: false,
    class_size: core::mem::size_of::<virtmcu_qom::qdev::SysBusDeviceClass>(),
    class_init: Some(actuator_class_init),
    class_base_init: None,
    class_data: ptr::null(),
    interfaces: ptr::null(),
};

declare_device_type!(VIRTMCU_ACTUATOR_TYPE_INIT, VIRTMCU_ACTUATOR_TYPE_INFO);

/* ── Internal Logic ───────────────────────────────────────────────────────── */

fn actuator_init_internal(
    parent: *mut VirtmcuActuatorQEMU,
    node_id: u32,
    transport: Arc<dyn virtmcu_api::DataTransport>,
) -> *mut VirtmcuActuatorState {
    let hb_topic = format!("sim/actuator/liveliness/{node_id}");
    let liveliness = transport.declare_liveliness(&hb_topic);

    Box::into_raw(Box::new(VirtmcuActuatorState {
        qemu_dev_ptr: parent,
        drain: VcpuDrain::new(),
        transport,
        seq: core::sync::atomic::AtomicU64::new(0),
        cond: virtmcu_qom::sync::Condvar::new(),
        wait_mutex: virtmcu_qom::sync::Mutex::new(()),
        _liveliness: liveliness,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_actuator_qemu_layout() {
        assert_eq!(
            core::mem::offset_of!(VirtmcuActuatorQEMU, parent_obj),
            0,
            "SysBusDevice must be the first field"
        );
    }
}
