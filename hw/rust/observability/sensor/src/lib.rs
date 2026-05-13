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
use core::ffi::{c_char, c_void};
use core::ptr;
use std::collections::HashMap;

use virtmcu_api::topics::sim_topic;
use virtmcu_qom::define_properties;
use virtmcu_qom::memory::{memory_region_init_io, MemoryRegion};
use virtmcu_qom::qdev::{sysbus_init_mmio, SysBusDevice};
use virtmcu_qom::qom::{Object, ObjectClass, TypeInfo};
use virtmcu_qom::sync::{BqlGuarded, SafeSubscription};
use virtmcu_qom::timer::{qemu_clock_get_ns, QomTimer, QEMU_CLOCK_VIRTUAL};
use virtmcu_qom::{define_prop_bool, define_prop_string, define_prop_uint32};

unsafe extern "C" fn allow_set_link(
    _obj: *mut Object,
    _name: *const core::ffi::c_char,
    _val: *mut Object,
    _errp: *mut *mut virtmcu_qom::error::Error,
) {
}

#[repr(C)]
#[derive(virtmcu_qom::MmioDevice)]
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

struct SensorFrame {
    delivery_vtime_ns: u64,
    sensor_id: u32,
    data: [f64; 8],
    data_size: u32,
}

pub struct VirtmcuSensorInner {
    map: HashMap<u32, SensorEntry>,
    rx_queue: alloc::vec::Vec<SensorFrame>,
    running: bool,
}

pub struct VirtmcuSensorState {
    parent_ptr: *mut VirtmcuSensorQEMU,
    inner: BqlGuarded<VirtmcuSensorInner>,
    drain: virtmcu_qom::sync::VcpuDrain,
    cond: virtmcu_qom::sync::Condvar,
    // virtmcu-allow: mutex reasoning="Required for Condvar::wait_yielding_bql"
    wait_mutex: virtmcu_qom::sync::Mutex<()>,
    rx_timer: Option<QomTimer>,
    pub _subscription: Option<SafeSubscription>,
    pub _liveliness: Option<Box<dyn virtmcu_api::LivelinessToken>>,
}

struct SensorEntry {
    data: [f64; 8],
    data_size: u32,
    new_data: bool,
}

const REG_SENSOR_ID: u64 = 0x00;
const REG_DATA_SIZE: u64 = 0x04;
const REG_SENS_GO: u64 = 0x08;
const REG_NEW_DATA: u64 = 0x0C;
const REG_DATA_START: u64 = 0x10;

const MAX_DATA_ELEMENTS: usize = 8;
const F64_SIZE_BYTES_USIZE: usize = 8;
const SENSOR_MMIO_REGION_SIZE: u64 = 256;
const DEFAULT_TIMEOUT_MS: u32 = 30000;

impl virtmcu_qom::device::MmioDevice for VirtmcuSensorState {
    fn read(&self, addr: u64, size: u32) -> virtmcu_qom::device::MmioResult<'_> {
        let s = unsafe { &mut *self.parent_ptr };
        let inner = self.inner.get_mut();
        if !inner.running {
            return virtmcu_qom::device::MmioResult::Ready(0);
        }
        let _guard = self.drain.acquire();

        if addr == REG_SENSOR_ID {
            virtmcu_qom::device::MmioResult::Ready(u64::from(s.sensor_id))
        } else if addr == REG_DATA_SIZE {
            virtmcu_qom::device::MmioResult::Ready(u64::from(s.data_size))
        } else if addr == REG_NEW_DATA {
            let mut ret = 0;
            if let Some(entry) = inner.map.get(&s.sensor_id) {
                if entry.new_data {
                    ret = 1;
                }
            }

            if ret == 0 {
                let sensor_id = s.sensor_id;
                let debug = s.debug;
                drop(inner);
                return virtmcu_qom::device::MmioResult::wait_for(
                    move || {
                        let i2 = self.inner.get();
                        if let Some(entry) = i2.map.get(&sensor_id) {
                            if entry.new_data {
                                return true;
                            }
                        }
                        false
                    },
                    move || {
                        if debug {
                            virtmcu_qom::sim_info!(
                                "sensor_read: REG_NEW_DATA (sensor_id={}) -> 1",
                                sensor_id
                            );
                        }
                        1
                    },
                );
            }

            if s.debug {
                virtmcu_qom::sim_info!(
                    "sensor_read: REG_NEW_DATA (sensor_id={}) -> {}",
                    s.sensor_id,
                    ret
                );
            }
            virtmcu_qom::device::MmioResult::Ready(ret)
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
                    if let (Some(dest), Some(src)) = (
                        ret_bytes.get_mut(..size as usize),
                        bytes.get(offset..offset + size as usize),
                    ) {
                        dest.copy_from_slice(src);
                        ret = u64::from_le_bytes(ret_bytes);
                    }
                }
            }
            virtmcu_qom::device::MmioResult::Ready(ret)
        } else {
            virtmcu_qom::device::MmioResult::Ready(0)
        }
    }

    fn write(&self, addr: u64, val: u64, _size: u32) {
        let s = unsafe { &mut *self.parent_ptr };
        let mut inner = self.inner.get_mut();
        if !inner.running {
            return;
        }
        let _guard = self.drain.acquire();

        if s.debug {
            virtmcu_qom::sim_info!("sensor_write: addr 0x{:x}, val {}", addr, val);
        }

        if addr == REG_SENSOR_ID {
            s.sensor_id = val as u32;
        } else if addr == REG_DATA_SIZE {
            s.data_size = val as u32;
        } else if addr == REG_SENS_GO {
            if let Some(entry) = inner.map.get_mut(&s.sensor_id) {
                s.data = entry.data;
                s.data_size = entry.data_size;
                entry.new_data = false;
                if s.debug {
                    virtmcu_qom::sim_info!(
                        "sensor_write: latched data for sensor_id {}",
                        s.sensor_id
                    );
                }
            } else if s.debug {
                virtmcu_qom::sim_info!("sensor_write: NO DATA for sensor_id {}", s.sensor_id);
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
/// This function is called by QEMU.
#[no_mangle]
pub unsafe extern "C" fn sensor_realize(dev: *mut c_void, errp: *mut *mut c_void) {
    virtmcu_qom::vlog!("SENSOR_REALIZE CALLED\n");
    let s = unsafe { &mut *(dev as *mut VirtmcuSensorQEMU) };

    if !s.rust_state.is_null() {
        return;
    }

    unsafe {
        memory_region_init_io(
            &raw mut s.mmio,
            dev as *mut Object,
            &raw const VIRTMCUSENSORQEMU_OPS,
            dev,
            c"virtmcu-sensor".as_ptr(),
            SENSOR_MMIO_REGION_SIZE,
        );
        sysbus_init_mmio(dev as *mut SysBusDevice, &raw mut s.mmio);
    }

    if s.transport_hub.is_null() {
        virtmcu_qom::error_setg!(
            errp,
            "Strict DI violation: sensor transport_hub link is required."
        );
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
    let transport_ref = unsafe { &*(ptr_u64 as *const Arc<dyn virtmcu_api::DataTransport>) };
    let transport = Arc::clone(transport_ref);

    s.rust_state = sensor_init_internal(s, s.node_id, transport);
    if s.rust_state.is_null() {
        virtmcu_qom::error_setg!(errp, "sensor: failed to initialize Rust backend");
    }
}

fn sensor_init_internal(
    s: &VirtmcuSensorQEMU,
    node_id: u32,
    transport: alloc::sync::Arc<dyn virtmcu_api::DataTransport>,
) -> *mut VirtmcuSensorState {
    let mut state_box = Box::new(VirtmcuSensorState {
        parent_ptr: core::ptr::from_ref::<VirtmcuSensorQEMU>(s).cast_mut(),
        inner: BqlGuarded::new(VirtmcuSensorInner {
            map: HashMap::new(),
            rx_queue: alloc::vec::Vec::new(),
            running: true,
        }),
        drain: virtmcu_qom::sync::VcpuDrain::new(),
        cond: virtmcu_qom::sync::Condvar::new(),
        wait_mutex: virtmcu_qom::sync::Mutex::new(()),
        rx_timer: None,
        _subscription: None,
        _liveliness: None,
    });

    let state_ptr = core::ptr::from_mut(&mut *state_box);
    let state_ptr_usize = state_ptr as usize;

    let node_id_str = node_id.to_string();
    let topic = sim_topic::sensor_data_wildcard(&node_id_str);

    let callback: virtmcu_api::DataCallback = Box::new(move |topic_str: &str, payload: &[u8]| {
        let state = unsafe { &mut *(state_ptr_usize as *mut VirtmcuSensorState) };
        let mut inner = state.inner.get_mut();
        if !inner.running {
            return;
        }

        if let Some((header, data_ptr)) = virtmcu_api::decode_frame(payload) {
            if let Some(sensor_id_str) = topic_str.split('/').next_back() {
                let id_str = if sensor_id_str.starts_with("resd_") {
                    sensor_id_str.rsplit_once('_').map_or(sensor_id_str, |(_, id)| id)
                } else {
                    sensor_id_str.strip_prefix("sensordata_").unwrap_or(sensor_id_str)
                };

                if let Ok(sensor_id) = id_str.parse::<u32>() {
                    let mut data = [0.0f64; MAX_DATA_ELEMENTS];
                    let mut count = 0;
                    for (d, chunk) in
                        data.iter_mut().zip(data_ptr.chunks_exact(F64_SIZE_BYTES_USIZE))
                    {
                        *d = f64::from_le_bytes(
                            chunk.try_into().unwrap_or([0u8; F64_SIZE_BYTES_USIZE]),
                        );
                        count += 1;
                    }
                    let data_size = count;

                    let vtime = header.delivery_vtime_ns();
                    inner.rx_queue.push(SensorFrame {
                        delivery_vtime_ns: vtime,
                        sensor_id,
                        data,
                        data_size,
                    });
                    inner.rx_queue.sort_by_key(|f| f.delivery_vtime_ns);

                    if let Some(timer) = &state.rx_timer {
                        timer.mod_ns(inner.rx_queue[0].delivery_vtime_ns as i64);
                    }
                }
            }
        }
    });

    let generation = alloc::sync::Arc::new(core::sync::atomic::AtomicU64::new(0)); // Generation 0
                                                                                   // virtmcu-allow: bql reasoning="SafeSubscription is the approved pattern for async transport subscribers to avoid BQL deadlocks"
    let sub = match SafeSubscription::new(transport.as_ref(), &topic, generation, callback) {
        Ok(s) => s,
        Err(e) => {
            virtmcu_qom::sim_err!("Failed to subscribe to topic {}: {}", topic, e);
            return ptr::null_mut();
        }
    };

    let hb_topic = format!("sim/sensor/liveliness/{node_id}");
    let liveliness = transport.declare_liveliness(&hb_topic);

    state_box._subscription = Some(sub);
    state_box._liveliness = liveliness;
    state_box.rx_timer = Some(unsafe {
        QomTimer::new(QEMU_CLOCK_VIRTUAL, sensor_rx_timer_cb, state_ptr as *mut core::ffi::c_void)
    });

    Box::into_raw(state_box)
}

extern "C" fn sensor_rx_timer_cb(opaque: *mut core::ffi::c_void) {
    let state = unsafe { &mut *(opaque as *mut VirtmcuSensorState) };
    let mut inner = state.inner.get_mut();
    if !inner.running {
        return;
    }

    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;
    virtmcu_qom::vlog!("sensor_rx_timer_cb: now={}, queue_len={}\n", now, inner.rx_queue.len());
    let mut notified = false;

    while !inner.rx_queue.is_empty() && inner.rx_queue[0].delivery_vtime_ns <= now {
        let frame = inner.rx_queue.remove(0);
        virtmcu_qom::vlog!(
            "sensor_rx_timer_cb: processing frame for vtime {}\n",
            frame.delivery_vtime_ns
        );
        inner.map.insert(
            frame.sensor_id,
            SensorEntry { data: frame.data, data_size: frame.data_size, new_data: true },
        );
        notified = true;
    }

    if notified {
        let _guard = state.wait_mutex.lock();
        state.cond.notify_all();
        virtmcu_qom::vlog!("sensor_rx_timer_cb: notified!\n");
    }

    if let Some(first) = inner.rx_queue.first() {
        if let Some(timer) = &state.rx_timer {
            timer.mod_ns(first.delivery_vtime_ns as i64);
        }
    }
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

impl Drop for VirtmcuSensorState {
    fn drop(&mut self) {
        self.inner.get_mut().running = false;
        self.cond.notify_all();
        self.drain.wait_for_drain(DEFAULT_TIMEOUT_MS);
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

virtmcu_qom::declare_device_type!(VIRTMCU_SENSOR_TYPE_INIT, VIRTMCU_SENSOR_TYPE_INFO);

/// # Safety
/// This function is called by QEMU.
#[no_mangle]
pub unsafe extern "C" fn sensor_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let dc = virtmcu_qom::device_class!(klass);
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
}

define_properties!(
    VIRTMCU_SENSOR_PROPERTIES,
    [
        define_prop_uint32!(c"node".as_ptr(), VirtmcuSensorQEMU, node_id, 0),
        define_prop_string!(c"topic-prefix".as_ptr(), VirtmcuSensorQEMU, topic_prefix),
        define_prop_bool!(c"debug".as_ptr(), VirtmcuSensorQEMU, debug, false),
    ]
);
