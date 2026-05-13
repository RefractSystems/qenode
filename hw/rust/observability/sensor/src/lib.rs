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

extern crate alloc;

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use core::ffi::{c_char, c_uint, c_void, CStr};
use core::ptr;
use std::collections::HashMap;
use std::sync::RwLock;
use virtmcu_api::FlatBufferStructExt;
use virtmcu_qom::memory::{
    memory_region_init_io, MemoryRegion, MemoryRegionOps, DEVICE_LITTLE_ENDIAN,
};
use virtmcu_qom::qdev::{sysbus_init_mmio, SysBusDevice};
use virtmcu_qom::qom::{Object, ObjectClass, TypeInfo};
use virtmcu_api::topics::sim_topic;
use virtmcu_qom::{
    declare_device_type, define_prop_string, define_prop_uint32, define_properties, device_class,
    error_setg,
};

#[repr(C)]
pub struct VirtmcuSensorQEMU {
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

    /* Latched Registers (vCPU state) */
    pub sensor_id: u32,
    pub data_size: u32,
    pub data: [f64; 8],
    pub new_data: u32,

    /* Rust state */
    pub rust_state: *mut VirtmcuSensorState,
}

#[derive(Clone, Default)]
struct SensorEntry {
    data: [f64; 8],
    data_size: u32,
    new_data: bool,
}

pub struct VirtmcuSensorState {
    shared: Arc<RwLock<HashMap<u32, SensorEntry>>>,
    _sub: virtmcu_qom::sync::SafeSubscription,
    pub _liveliness: Option<alloc::boxed::Box<dyn virtmcu_api::LivelinessToken>>,
}

const REG_SENSOR_ID: u64 = 0x00;
const REG_DATA_SIZE: u64 = 0x04;
const REG_NEW_DATA: u64 = 0x08;
const REG_DATA_START: u64 = 0x10;
const MAX_DATA_ELEMENTS: usize = 8;
const F64_SIZE_BYTES: u64 = core::mem::size_of::<f64>() as u64;

/// # Safety
/// This function is called by QEMU.
#[no_mangle]
pub unsafe extern "C" fn sensor_read(opaque: *mut c_void, addr: u64, size: c_uint) -> u64 {
    let s = unsafe { &mut *(opaque as *mut VirtmcuSensorQEMU) };

    if addr == REG_SENSOR_ID {
        u64::from(s.sensor_id)
    } else if addr == REG_DATA_SIZE {
        u64::from(s.data_size)
    } else if addr == REG_NEW_DATA {
        let ret = u64::from(s.new_data);
        s.new_data = 0; // Clear on read
        ret
    } else if (REG_DATA_START..REG_DATA_START + (MAX_DATA_ELEMENTS as u64) * F64_SIZE_BYTES)
        .contains(&addr)
    {
        let idx = ((addr - REG_DATA_START) / F64_SIZE_BYTES) as usize;
        let offset = ((addr - REG_DATA_START) % F64_SIZE_BYTES) as usize;
        let mut ret: u64 = 0;
        if offset + (size as usize) <= (F64_SIZE_BYTES as usize) {
            let bytes = s.data.get(idx).expect("idx out of bounds").to_le_bytes();
            let mut ret_bytes = [0u8; core::mem::size_of::<f64>()];
            if let (Some(dest), Some(src)) =
                (ret_bytes.get_mut(..size as usize), bytes.get(offset..offset + size as usize))
            {
                dest.copy_from_slice(src);
                ret = u64::from_le_bytes(ret_bytes);
            }
        }
        ret
    } else {
        0
    }
}

/// # Safety
/// This function is called by QEMU.
#[no_mangle]
pub unsafe extern "C" fn sensor_write(opaque: *mut c_void, addr: u64, val: u64, _size: c_uint) {
    let s = unsafe { &mut *(opaque as *mut VirtmcuSensorQEMU) };

    if addr == REG_SENSOR_ID {
        s.sensor_id = u32::try_from(val).expect("sensor_id truncated");
        if !s.rust_state.is_null() {
            let rs = unsafe { &*s.rust_state };
            // Latch the current data for this sensor_id
            if let Ok(map) = rs.shared.read() {
                if let Some(entry) = map.get(&s.sensor_id) {
                    s.data = entry.data;
                    s.data_size = entry.data_size;
                    s.new_data = if entry.new_data { 1 } else { 0 };
                } else {
                    s.data = [0.0; 8];
                    s.data_size = 0;
                    s.new_data = 0;
                }
            }
        }
    }
}

static VIRTMCU_SENSOR_OPS: MemoryRegionOps = MemoryRegionOps {
    read: Some(sensor_read),
    write: Some(sensor_write),
    read_with_attrs: ptr::null(),
    write_with_attrs: ptr::null(),
    endianness: DEVICE_LITTLE_ENDIAN,
    _padding1: [0; 4],
    valid: virtmcu_qom::memory::MemoryRegionValidRange {
        min_access_size: 1,
        max_access_size: F64_SIZE_BYTES as u32,
        unaligned: false,
        _padding: [0; 7],
        accepts: ptr::null(),
    },
    impl_: virtmcu_qom::memory::MemoryRegionImplRange {
        min_access_size: 1,
        max_access_size: F64_SIZE_BYTES as u32,
        unaligned: false,
        _padding: [0; 7],
    },
};

/// # Safety
/// This function is called by QEMU.
#[no_mangle]
pub unsafe extern "C" fn sensor_realize(dev: *mut c_void, errp: *mut *mut c_void) {
    const SENSOR_MMIO_SIZE: u64 = 0x1000;
    let s = unsafe { &mut *(dev as *mut VirtmcuSensorQEMU) };

    unsafe {
        memory_region_init_io(
            &raw mut s.mmio,
            dev as *mut Object,
            &raw const VIRTMCU_SENSOR_OPS,
            dev,
            c"sensor".as_ptr(),
            SENSOR_MMIO_SIZE,
        );
        sysbus_init_mmio(dev as *mut SysBusDevice, &raw mut s.mmio);
    }

    if s.transport_hub.is_null() {
        error_setg!(errp, "Strict DI violation: sensor transport_hub link is required.");
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
        error_setg!(errp, "Strict DI violation: failed to acquire transport from hub.");
        return;
    }
    let transport_ref =
        unsafe { &*(ptr_u64 as *const alloc::sync::Arc<dyn virtmcu_api::DataTransport>) };
    let transport = alloc::sync::Arc::clone(transport_ref);

    s.rust_state = sensor_init_internal(s, s.node_id, transport);
}

/// # Safety
/// This function is called by QEMU.
#[no_mangle]
pub unsafe extern "C" fn sensor_instance_finalize(obj: *mut Object) {
    let s = unsafe { &mut *(obj as *mut VirtmcuSensorQEMU) };
    if !s.rust_state.is_null() {
        unsafe {
            drop(Box::from_raw(s.rust_state));
        }
        s.rust_state = ptr::null_mut();
    }
}

/// # Safety
/// This function is called by QEMU.
#[no_mangle]
pub unsafe extern "C" fn sensor_instance_init(obj: *mut Object) {
    let s = unsafe { &mut *(obj as *mut VirtmcuSensorQEMU) };
    s.topic_prefix = ptr::null_mut();
    s.transport = ptr::null_mut();
}

define_properties!(
    VIRTMCU_SENSOR_PROPERTIES,
    [
        define_prop_uint32!(c"node".as_ptr(), VirtmcuSensorQEMU, node_id, 0),
        define_prop_string!(c"router".as_ptr(), VirtmcuSensorQEMU, router),
        define_prop_string!(c"topic-prefix".as_ptr(), VirtmcuSensorQEMU, topic_prefix),
        virtmcu_qom::define_prop_bool!(c"debug".as_ptr(), VirtmcuSensorQEMU, debug, false),
    ]
);

/// # Safety
/// This function is called by QEMU.
#[no_mangle]
pub unsafe extern "C" fn sensor_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let dc = device_class!(klass);
    unsafe {
        (*dc).realize = Some(sensor_realize);
        (*dc).user_creatable = true;
    }
    virtmcu_qom::device_class_set_props!(dc, VIRTMCU_SENSOR_PROPERTIES);

    unsafe {
        virtmcu_qom::qom::object_class_property_add_link(
            klass,
            c"transport".as_ptr(),
            c"virtmcu-transport-hub".as_ptr(),
            core::mem::offset_of!(VirtmcuSensorQEMU, transport_hub) as isize,
            Some(allow_set_link),
            virtmcu_qom::qom::OBJ_PROP_LINK_STRONG,
        );
    }
}

#[used]
static VIRTMCU_SENSOR_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"sensor".as_ptr(),
    parent: c"sys-bus-device".as_ptr(),
    instance_size: core::mem::size_of::<VirtmcuSensorQEMU>(),
    instance_align: 0,
    instance_init: Some(sensor_instance_init),
    instance_post_init: None,
    instance_finalize: Some(sensor_instance_finalize),
    abstract_: false,
    class_size: core::mem::size_of::<virtmcu_qom::qdev::SysBusDeviceClass>(),
    class_init: Some(sensor_class_init),
    class_base_init: None,
    class_data: ptr::null(),
    interfaces: ptr::null(),
};

declare_device_type!(VIRTMCU_SENSOR_TYPE_INIT, VIRTMCU_SENSOR_TYPE_INFO);

/* ── Internal Logic ───────────────────────────────────────────────────────── */

use virtmcu_api::ZenohFrameHeader;
use virtmcu_api::ZENOH_FRAME_HEADER_SIZE;

fn sensor_init_internal(
    _dev: *mut VirtmcuSensorQEMU,
    node_id: u32,
    transport: Arc<dyn virtmcu_api::DataTransport>,
) -> *mut VirtmcuSensorState {
    let shared = Arc::new(RwLock::new(HashMap::new()));
    let shared_bg = Arc::clone(&shared);
    let node_id_str = node_id.to_string();
    let topic = format!("{}/{}", sim_topic::sensor_data(&node_id_str, 0).rsplit_once('/').unwrap().0, "**");

    // We construct a generation tracker to satisfy SafeSubscription.
    let generation = Arc::new(core::sync::atomic::AtomicU64::new(0));

    let callback: virtmcu_api::DataCallback = Box::new(move |topic_str: &str, payload: &[u8]| {
        if payload.len() < ZENOH_FRAME_HEADER_SIZE {
            return;
        }

        if let Some(_header) = ZenohFrameHeader::unpack_slice(payload) {
            let data_bytes = &payload[ZENOH_FRAME_HEADER_SIZE..];
            let mut data = [0.0; 8];
            let mut data_size = 0;
            for (i, chunk) in
                data_bytes.chunks_exact(F64_SIZE_BYTES as usize).enumerate().take(MAX_DATA_ELEMENTS)
            {
                if let Ok(arr) = chunk.try_into() {
                    data[i] = f64::from_le_bytes(arr);
                    data_size += 1;
                }
            }

            if let Some(sensor_id_str) = topic_str.split('/').next_back() {
                if let Ok(sensor_id) = sensor_id_str.parse::<u32>() {
                    if let Ok(mut map) = shared_bg.write() {
                        map.insert(sensor_id, SensorEntry { data, data_size, new_data: true });
                    }
                }
            }
        }
    });

    let sub =
        virtmcu_qom::sync::SafeSubscription::new(transport.as_ref(), &topic, generation, callback)
            .expect("SafeSubscription creation failed");

    let hb_topic = format!("sim/sensor/liveliness/{node_id}");
    let liveliness = transport.declare_liveliness(&hb_topic);

    Box::into_raw(Box::new(VirtmcuSensorState { shared, _sub: sub, _liveliness: liveliness }))
}

#[cfg(test)]
#[allow(clippy::magic_numbers)] // virtmcu-allow: allow reasoning="Tests require specific magic numbers"
mod tests {
    use super::*;

    const TEST_SENSOR_ID: u32 = 42;
    const TEST_DATA_SIZE: u32 = 2;
    const TEST_NEW_DATA: u32 = 1;
    const ACCESS_SIZE_4: u32 = 4;
    const ACCESS_SIZE_8: u32 = 8;

    const TEST_VAL_1: f64 = 1.0;
    const TEST_VAL_2: f64 = 2.0;

    #[test]
    fn test_sensor_qemu_layout() {
        assert_eq!(
            core::mem::offset_of!(VirtmcuSensorQEMU, parent_obj),
            0,
            "SysBusDevice must be the first field"
        );
    }

    #[test]
    fn test_sensor_mmio_read() {
        let mut device = VirtmcuSensorQEMU {
            parent_obj: unsafe { core::mem::zeroed() },
            mmio: unsafe { core::mem::zeroed() },
            node_id: 0,
            transport: ptr::null_mut(),
            router: ptr::null_mut(),
            topic_prefix: ptr::null_mut(),
            debug: false,
            transport_hub: ptr::null_mut(),
            sensor_id: TEST_SENSOR_ID,
            data_size: TEST_DATA_SIZE,
            data: [TEST_VAL_1, TEST_VAL_2, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            new_data: TEST_NEW_DATA,
            rust_state: ptr::null_mut(),
        };
        let opaque = &mut device as *mut _ as *mut c_void;

        assert_eq!(
            unsafe { sensor_read(opaque, REG_SENSOR_ID, ACCESS_SIZE_4) },
            TEST_SENSOR_ID as u64
        );
        assert_eq!(
            unsafe { sensor_read(opaque, REG_DATA_SIZE, ACCESS_SIZE_4) },
            TEST_DATA_SIZE as u64
        );
        assert_eq!(
            unsafe { sensor_read(opaque, REG_NEW_DATA, ACCESS_SIZE_4) },
            TEST_NEW_DATA as u64
        );
        assert_eq!(device.new_data, 0); // Cleared on read

        let val1_u64 = unsafe { sensor_read(opaque, REG_DATA_START, ACCESS_SIZE_8) };
        assert_eq!(f64::from_le_bytes(val1_u64.to_le_bytes()), TEST_VAL_1);

        let val2_u64 =
            unsafe { sensor_read(opaque, REG_DATA_START + ACCESS_SIZE_8 as u64, ACCESS_SIZE_8) };
        assert_eq!(f64::from_le_bytes(val2_u64.to_le_bytes()), TEST_VAL_2);
    }
}
