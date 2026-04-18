#![allow(unused_variables)]
#![allow(clippy::all)]
#![allow(
    clippy::missing_safety_doc,
    clippy::collapsible_match,
    dead_code,
    unused_imports,
    clippy::needless_return,
    clippy::manual_range_contains,
    clippy::single_component_path_imports,
    clippy::len_zero,
    clippy::while_immutable_condition
)]

use core::ffi::{c_char, c_uint, c_void};
use std::ffi::{CStr, CString};
use std::ptr;
use std::sync::Arc;
use virtmcu_qom::error::Error;
use virtmcu_qom::memory::{
    memory_region_init_io, MemoryRegion, MemoryRegionOps, DEVICE_LITTLE_ENDIAN,
};
use virtmcu_qom::qdev::{sysbus_init_mmio, DeviceClass, SysBusDevice};
use virtmcu_qom::qom::{Object, ObjectClass, TypeInfo};
use virtmcu_qom::{
    declare_device_type, define_prop_string, define_prop_uint32, define_properties, device_class,
    device_class_set_props, error_setg,
};
use zenoh::Session;
use zenoh::Wait;

#[repr(C)]
pub struct ZenohActuatorQEMU {
    pub parent_obj: SysBusDevice,
    pub mmio: MemoryRegion,

    /* Properties */
    pub node_id: u32,
    pub router: *mut c_char,
    pub topic_prefix: *mut c_char,

    /* Registers */
    pub actuator_id: u32,
    pub data_size: u32,
    pub data: [f64; 8],

    /* Rust state */
    pub rust_state: *mut ZenohActuatorState,
}

pub struct ZenohActuatorState {
    session: Session,
    node_id: u32,
    topic_prefix: String,
}

const REG_ACTUATOR_ID: u64 = 0x00;
const REG_DATA_SIZE: u64 = 0x04;
const REG_GO: u64 = 0x08;
const REG_DATA_START: u64 = 0x10;

unsafe extern "C" fn zenoh_actuator_read(opaque: *mut c_void, addr: u64, size: c_uint) -> u64 {
    let s = &mut *(opaque as *mut ZenohActuatorQEMU);

    if addr == REG_ACTUATOR_ID {
        s.actuator_id as u64
    } else if addr == REG_DATA_SIZE {
        s.data_size as u64
    } else if addr >= REG_DATA_START && addr < REG_DATA_START + 8 * 8 {
        let idx = ((addr - REG_DATA_START) / 8) as usize;
        let offset = ((addr - REG_DATA_START) % 8) as usize;
        let mut ret: u64 = 0;
        if offset + (size as usize) <= 8 {
            unsafe {
                ptr::copy_nonoverlapping(
                    (s.data.as_ptr().add(idx) as *const u8).add(offset),
                    &mut ret as *mut u64 as *mut u8,
                    size as usize,
                );
            }
        }
        ret
    } else {
        0
    }
}

unsafe extern "C" fn zenoh_actuator_write(opaque: *mut c_void, addr: u64, val: u64, size: c_uint) {
    let s = &mut *(opaque as *mut ZenohActuatorQEMU);

    if addr == REG_ACTUATOR_ID {
        s.actuator_id = val as u32;
    } else if addr == REG_DATA_SIZE {
        s.data_size = val as u32;
        if s.data_size > 8 {
            s.data_size = 8;
        }
    } else if addr == REG_GO {
        if val == 1 && !s.rust_state.is_null() {
            let rs = &*s.rust_state;
            zenoh_actuator_publish(rs, s.actuator_id, s.data_size, &s.data);
        }
    }
 else if addr >= REG_DATA_START && addr < REG_DATA_START + 8 * 8 {
        let idx = ((addr - REG_DATA_START) / 8) as usize;
        let offset = ((addr - REG_DATA_START) % 8) as usize;
        if offset + (size as usize) <= 8 {
            unsafe {
                ptr::copy_nonoverlapping(
                    &val as *const u64 as *const u8,
                    (s.data.as_mut_ptr().add(idx) as *mut u8).add(offset),
                    size as usize,
                );
            }
        }
    }
}

static ZENOH_ACTUATOR_OPS: MemoryRegionOps = MemoryRegionOps {
    read: Some(zenoh_actuator_read),
    write: Some(zenoh_actuator_write),
    read_with_attrs: ptr::null(),
    write_with_attrs: ptr::null(),
    endianness: DEVICE_LITTLE_ENDIAN,
    _padding1: [0; 4],
    valid: virtmcu_qom::memory::MemoryRegionValidRange {
        min_access_size: 4,
        max_access_size: 8,
        unaligned: false,
        _padding: [0; 7],
        accepts: ptr::null(),
    },
    impl_: virtmcu_qom::memory::MemoryRegionImplRange {
        min_access_size: 1,
        max_access_size: 8,
        unaligned: false,
        _padding: [0; 7],
    },
};

unsafe extern "C" fn zenoh_actuator_realize(dev: *mut c_void, errp: *mut *mut c_void) {
    let s = &mut *(dev as *mut ZenohActuatorQEMU);

    memory_region_init_io(
        &mut s.mmio,
        dev as *mut Object,
        &ZENOH_ACTUATOR_OPS,
        dev,
        c"zenoh-actuator".as_ptr(),
        0x100,
    );
    sysbus_init_mmio(dev as *mut SysBusDevice, &mut s.mmio);

    let router_ptr = if s.router.is_null() {
        ptr::null()
    } else {
        s.router as *const c_char
    };

    let prefix = if s.topic_prefix.is_null() {
        "firmware/control".to_string()
    } else {
        unsafe {
            CStr::from_ptr(s.topic_prefix)
                .to_string_lossy()
                .into_owned()
        }
    };

    s.rust_state = zenoh_actuator_init_internal(s.node_id, router_ptr, prefix);
    if s.rust_state.is_null() {
        virtmcu_qom::error_setg!(
            errp as *mut *mut Error,
            c"zenoh-actuator: failed to initialize Rust backend".as_ptr()
        );
        return;
    }
}

unsafe extern "C" fn zenoh_actuator_instance_finalize(obj: *mut Object) {
    let s = &mut *(obj as *mut ZenohActuatorQEMU);
    if !s.rust_state.is_null() {
        unsafe {
            drop(Box::from_raw(s.rust_state));
        }
        s.rust_state = ptr::null_mut();
    }
}

unsafe extern "C" fn zenoh_actuator_instance_init(obj: *mut Object) {
    let s = &mut *(obj as *mut ZenohActuatorQEMU);
    s.topic_prefix = ptr::null_mut();
}

define_properties!(
    ZENOH_ACTUATOR_PROPERTIES,
    [
        define_prop_uint32!(c"node".as_ptr(), ZenohActuatorQEMU, node_id, 0),
        define_prop_string!(c"router".as_ptr(), ZenohActuatorQEMU, router),
        define_prop_string!(c"topic-prefix".as_ptr(), ZenohActuatorQEMU, topic_prefix),
    ]
);

unsafe extern "C" fn zenoh_actuator_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let dc = device_class!(klass);
    unsafe {
        (*dc).realize = Some(zenoh_actuator_realize);
        (*dc).user_creatable = true;
    }
    virtmcu_qom::device_class_set_props!(dc, ZENOH_ACTUATOR_PROPERTIES);
}

static ZENOH_ACTUATOR_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"zenoh-actuator".as_ptr(),
    parent: c"sys-bus-device".as_ptr(),
    instance_size: std::mem::size_of::<ZenohActuatorQEMU>(),
    instance_align: 0,
    instance_init: Some(zenoh_actuator_instance_init),
    instance_post_init: None,
    instance_finalize: Some(zenoh_actuator_instance_finalize),
    abstract_: false,
    class_size: 0,
    class_init: Some(zenoh_actuator_class_init),
    class_base_init: None,
    class_data: ptr::null(),
    interfaces: ptr::null(),
};

declare_device_type!(zenoh_actuator_type_init, ZENOH_ACTUATOR_TYPE_INFO);

/* ── Internal Logic ───────────────────────────────────────────────────────── */

fn zenoh_actuator_init_internal(
    node_id: u32,
    router: *const c_char,
    topic_prefix: String,
) -> *mut ZenohActuatorState {
    let session = unsafe {
        match virtmcu_zenoh::open_session(router) {
            Ok(s) => s,
            Err(_) => return ptr::null_mut(),
        }
    };

    Box::into_raw(Box::new(ZenohActuatorState {
        session,
        node_id,
        topic_prefix,
    }))
}

fn zenoh_actuator_publish(
    state: &ZenohActuatorState,
    actuator_id: u32,
    data_size: u32,
    data: &[f64; 8],
) {
    let vtime_ns = unsafe {
        virtmcu_qom::timer::qemu_clock_get_ns(virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL)
    } as u64;

    let topic = format!("{}/{}/{}", state.topic_prefix, state.node_id, actuator_id);
    let mut payload = Vec::with_capacity(8 + (data_size as usize) * 8);
    payload.extend_from_slice(&vtime_ns.to_le_bytes());
    for i in 0..(data_size as usize) {
        payload.extend_from_slice(&data[i].to_le_bytes());
    }

    virtmcu_qom::vlog!("[zenoh-actuator] Publishing to {} (size={})\n", topic, payload.len());
    let _ = state.session.put(topic, payload).wait();
}
