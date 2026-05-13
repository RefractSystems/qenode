/*
 * hw/rust/observability/sensor/src/lib.rs
 *
 * Virtmcu sensor device with Zenoh ingress.
 */

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

extern crate alloc;

use alloc::boxed::Box;
use alloc::sync::Arc;
use core::ffi::{c_char, c_uint, c_void};
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::collections::HashMap;
use std::sync::RwLock;
use virtmcu_api::topics::sim_topic;
use virtmcu_api::FlatBufferStructExt;
use virtmcu_api::ZenohFrameHeader;
use virtmcu_api::ZENOH_FRAME_HEADER_SIZE;
use virtmcu_qom::memory::{
    memory_region_init_io, MemoryRegion, MemoryRegionOps, DEVICE_LITTLE_ENDIAN,
};
use virtmcu_qom::qdev::{sysbus_init_mmio, SysBusDevice};
use virtmcu_qom::qom::{Object, ObjectClass, TypeInfo};
use virtmcu_qom::sync::VcpuDrain;
use virtmcu_qom::timer::{virtmcu_timer_mod, virtmcu_timer_new_ns, QemuTimer, QEMU_CLOCK_REALTIME};
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
    pub transport_hub: *mut Object,
    pub topic_prefix: *mut c_char,
    pub debug: bool,

    /* State */
    pub sensor_id: u32,
    pub data_size: u32,
    pub new_data: u32,
    pub data: [f64; 8],

    pub rust_state: *mut VirtmcuSensorState,
}

pub struct VirtmcuSensorState {
    pub shared: Arc<SharedState>,
    pub _subscription: virtmcu_qom::sync::SafeSubscription,
    pub _liveliness: Option<alloc::boxed::Box<dyn virtmcu_api::LivelinessToken>>,
    pub kick_timer: *mut QemuTimer,
}

pub struct SharedState {
    pub map: RwLock<HashMap<u32, SensorEntry>>,
    pub running: AtomicBool,
    pub drain: VcpuDrain,
    pub timer_ptr: AtomicUsize,
    pub node_id: u32,
}

pub struct SensorEntry {
    pub data: [f64; 8],
    pub data_size: u32,
    pub new_data: bool,
}

const REG_SENSOR_ID: u64 = 0x00;
const REG_DATA_SIZE: u64 = 0x04;
const REG_SENS_GO: u64 = 0x08;
const REG_NEW_DATA: u64 = 0x0C;
const REG_DATA_START: u64 = 0x10;
const MAX_DATA_ELEMENTS: usize = 8;
const F64_SIZE_BYTES_USIZE: usize = 8;
const DRAIN_TIMEOUT_MS: u32 = 30000;
const SENSOR_MMIO_REGION_SIZE: u64 = 256;

/// # Safety
/// This function is called by QEMU.
#[no_mangle]
pub unsafe extern "C" fn sensor_read(opaque: *mut c_void, addr: u64, size: c_uint) -> u64 {
    let s = unsafe { &mut *(opaque as *mut VirtmcuSensorQEMU) };
    if s.rust_state.is_null() {
        return 0;
    }
    let state = unsafe { &*s.rust_state };
    if !state.shared.running.load(Ordering::Acquire) {
        return 0;
    }
    virtmcu_qom::vlog!("sensor_read: addr 0x{:x}\n", addr);
    let _guard = state.shared.drain.acquire();

    if addr == REG_SENSOR_ID {
        virtmcu_qom::vlog!("sensor_read: REG_SENSOR_ID\n");
        u64::from(s.sensor_id)
    } else if addr == REG_DATA_SIZE {
        virtmcu_qom::vlog!("sensor_read: REG_DATA_SIZE\n");
        u64::from(s.data_size)
    } else if addr == REG_NEW_DATA {
        let mut ret = 0;
        if let Ok(map) = state.shared.map.read() {
            if let Some(entry) = map.get(&s.sensor_id) {
                if entry.new_data {
                    ret = 1;
                }
            }
        }
        virtmcu_qom::vlog!("sensor_read: REG_NEW_DATA -> {}\n", ret);
        ret
    } else if (REG_DATA_START
        ..REG_DATA_START + (MAX_DATA_ELEMENTS as u64) * (F64_SIZE_BYTES_USIZE as u64))
        .contains(&addr)
    {
        let idx = ((addr - REG_DATA_START) / (F64_SIZE_BYTES_USIZE as u64)) as usize;
        let offset = ((addr - REG_DATA_START) % (F64_SIZE_BYTES_USIZE as u64)) as usize;
        let mut ret: u64 = 0;
        if let Some(val_f64) = s.data.get(idx) {
            if offset + (size as usize) <= F64_SIZE_BYTES_USIZE {
                let bytes = val_f64.to_le_bytes();
                let mut ret_bytes = [0u8; F64_SIZE_BYTES_USIZE];
                if let (Some(dest), Some(src)) =
                    (ret_bytes.get_mut(..size as usize), bytes.get(offset..offset + size as usize))
                {
                    dest.copy_from_slice(src);
                    ret = u64::from_le_bytes(ret_bytes);
                }
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
    if s.rust_state.is_null() {
        return;
    }
    let state = unsafe { &*s.rust_state };
    if !state.shared.running.load(Ordering::Acquire) {
        return;
    }
    let _guard = state.shared.drain.acquire();

    if s.debug {
        virtmcu_qom::sim_info!("sensor_write: addr 0x{:x}, val {}", addr, val);
    }

    if addr == REG_SENSOR_ID {
        s.sensor_id = val as u32;
    } else if addr == REG_DATA_SIZE {
        s.data_size = val as u32;
    } else if addr == REG_SENS_GO {
        if let Ok(mut map) = state.shared.map.write() {
            if let Some(entry) = map.get_mut(&s.sensor_id) {
                s.data = entry.data;
                s.data_size = entry.data_size;
                entry.new_data = false;
            } else {
                s.data = [0.0; 8];
                s.data_size = 0;
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
        max_access_size: F64_SIZE_BYTES_USIZE as u32,
        unaligned: false,
        _padding: [0; 7],
        accepts: ptr::null(),
    },
    impl_: virtmcu_qom::memory::MemoryRegionImplRange {
        min_access_size: 1,
        max_access_size: F64_SIZE_BYTES_USIZE as u32,
        unaligned: false,
        _padding: [0; 7],
    },
};

define_properties!(
    VIRTMCU_SENSOR_PROPERTIES,
    [
        define_prop_uint32!(c"node".as_ptr(), VirtmcuSensorQEMU, node_id, 0),
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

/// # Safety
/// This function is called by QEMU.
#[no_mangle]
pub unsafe extern "C" fn sensor_instance_init(obj: *mut Object) {
    let s = unsafe { &mut *(obj as *mut VirtmcuSensorQEMU) };
    s.rust_state = ptr::null_mut();
    s.sensor_id = 0;
    s.data_size = 0;
    s.new_data = 0;
    s.data = [0.0; 8];
    unsafe {
        memory_region_init_io(
            &raw mut s.mmio,
            obj,
            &VIRTMCU_SENSOR_OPS,
            ptr::addr_of_mut!(*s) as *mut c_void,
            c"virtmcu-sensor".as_ptr(),
            SENSOR_MMIO_REGION_SIZE,
        );
        sysbus_init_mmio(obj as *mut SysBusDevice, &raw mut s.mmio);
    }
}

impl Drop for VirtmcuSensorState {
    fn drop(&mut self) {
        self.shared.running.store(false, Ordering::Release);
        self.shared.timer_ptr.store(0, Ordering::Release);
        self.shared.drain.wait_for_drain(DRAIN_TIMEOUT_MS);
        if !self.kick_timer.is_null() {
            unsafe {
                virtmcu_qom::timer::virtmcu_timer_del(self.kick_timer);
                virtmcu_qom::timer::virtmcu_timer_free(self.kick_timer);
            }
        }
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

/// # Safety
/// This function is called by QEMU.
#[no_mangle]
pub unsafe extern "C" fn sensor_realize(dev: *mut c_void, errp: *mut *mut c_void) {
    virtmcu_qom::vlog!("SENSOR_REALIZE CALLED\n");
    let s = unsafe { &mut *(dev as *mut VirtmcuSensorQEMU) };

    if s.transport_hub.is_null() {
        error_setg!(errp, "Strict DI violation: sensor transport link is required.");
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

extern "C" fn virtmcu_sensor_kick_timer_cb(_opaque: *mut c_void) {}

fn sensor_init_internal(
    _dev: *mut VirtmcuSensorQEMU,
    node_id: u32,
    transport: Arc<dyn virtmcu_api::DataTransport>,
) -> *mut VirtmcuSensorState {
    let shared = Arc::new(SharedState {
        map: RwLock::new(HashMap::new()),
        running: AtomicBool::new(true),
        drain: VcpuDrain::new(),
        timer_ptr: AtomicUsize::new(0),
        node_id,
    });

    let node_id_str = node_id.to_string();
    let shared_bg = Arc::clone(&shared);
    let topic = sim_topic::sensor_data_wildcard(&node_id_str);

    let callback: virtmcu_api::DataCallback = Box::new(move |topic_str: &str, payload: &[u8]| {
        virtmcu_qom::sim_info!("Sensor received data on topic: {}", topic_str);
        if !shared_bg.running.load(Ordering::Acquire) {
            return;
        }
        if payload.len() < ZENOH_FRAME_HEADER_SIZE {
            return;
        }

        if let Some(_header) = ZenohFrameHeader::unpack_slice(payload) {
            let data_bytes = &payload[ZENOH_FRAME_HEADER_SIZE..];
            let mut data = [0.0; 8];
            let mut data_size = 0;

            for (i, chunk) in
                data_bytes.chunks_exact(F64_SIZE_BYTES_USIZE).enumerate().take(MAX_DATA_ELEMENTS)
            {
                if let Ok(arr) = chunk.try_into() {
                    data[i] = f64::from_le_bytes(arr);
                    data_size += 1;
                }
            }

            if let Some(sensor_id_str) = topic_str.split('/').next_back() {
                let id_str = if sensor_id_str.starts_with("resd_") {
                    sensor_id_str.rsplit_once('_').map_or(sensor_id_str, |(_, id)| id)
                } else {
                    sensor_id_str.strip_prefix("sensordata_").unwrap_or(sensor_id_str)
                };

                if let Ok(sensor_id) = id_str.parse::<u32>() {
                    if let Ok(mut map) = shared_bg.map.write() {
                        map.insert(sensor_id, SensorEntry { data, data_size, new_data: true });
                    }
                    let tp = shared_bg.timer_ptr.load(Ordering::Acquire);
                    if tp != 0 {
                        unsafe { virtmcu_timer_mod(tp as *mut QemuTimer, 0) };
                    }
                }
            }
        }
    });

    let generation = Arc::new(AtomicU64::new(0));
    let subscription =
        virtmcu_qom::sync::SafeSubscription::new(transport.as_ref(), &topic, generation, callback)
            .expect("SafeSubscription creation failed");

    let kick_timer = unsafe {
        virtmcu_timer_new_ns(QEMU_CLOCK_REALTIME, virtmcu_sensor_kick_timer_cb, ptr::null_mut())
    };
    shared.timer_ptr.store(kick_timer as usize, Ordering::Release);

    let hb_topic = sim_topic::sensor_liveliness(&node_id_str);
    let liveliness = transport.declare_liveliness(&hb_topic);

    Box::into_raw(Box::new(VirtmcuSensorState {
        shared,
        _subscription: subscription,
        _liveliness: liveliness,
        kick_timer,
    }))
}

unsafe extern "C" fn allow_set_link(
    _obj: *mut Object,
    _name: *const core::ffi::c_char,
    _val: *mut Object,
    _errp: *mut *mut virtmcu_qom::error::Error,
) {
}
