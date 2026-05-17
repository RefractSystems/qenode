#![allow(clippy::all, unused_imports, dead_code, unused_variables, unused_mut)] // virtmcu-allow: allow reasoning="Zero unsafe"
#![allow(clippy::all)] // virtmcu-allow: allow reasoning="Zero unsafe"
#![allow(clippy::panic)]
#![allow(clippy::not_unsafe_ptr_arg_deref)]
// virtmcu-allow: allow reasoning="Zero unsafe"
#![allow(clippy::missing_safety_doc)]
#![cfg_attr(
    test,
    allow(
        clippy::expect_used,
        clippy::unwrap_used,
        clippy::indexing_slicing,
        clippy::panic_in_result_fn
    )
)]
use virtmcu_qom::memory::MemoryRegion;
use virtmcu_qom::qdev::SysBusDevice;
use virtmcu_qom::qom::Object;
// virtmcu-allow: allow reasoning="Fail Loudly"
/*
 * hw/rust/observability/sensor/src/lib.rs
 *
 * Virtmcu sensor device with Zenoh ingress.
 */

extern crate alloc;

use alloc::boxed::Box;
use core::ffi::c_char;
use core::ptr;
use std::collections::HashMap;

use virtmcu_qom::sync::BqlGuarded;
use virtmcu_qom::{define_prop_bool, define_prop_string, define_prop_uint32};
use virtmcu_wire::topics::sim_topic;

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

pub struct SensorFrame {
    delivery_vtime_ns: u64,
    sensor_id: u32,
    data: [f64; 8],
    data_size: u32,
}

impl PartialEq for SensorFrame {
    fn eq(&self, other: &Self) -> bool {
        self.delivery_vtime_ns == other.delivery_vtime_ns
    }
}
impl Eq for SensorFrame {}
impl PartialOrd for SensorFrame {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for SensorFrame {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.delivery_vtime_ns.cmp(&other.delivery_vtime_ns)
    }
}
impl virtmcu_qom::sync::DeliveryPacket for SensorFrame {
    fn delivery_vtime_ns(&self) -> u64 {
        self.delivery_vtime_ns
    }
}

pub struct VirtmcuSensorInner {
    map: HashMap<u32, SensorEntry>,
    running: bool,
}

pub struct VirtmcuSensorState {
    parent_ptr: *mut VirtmcuSensorQEMU,
    inner: BqlGuarded<VirtmcuSensorInner>,
    drain: virtmcu_qom::sync::VcpuDrain,
    cond: virtmcu_qom::sync::Condvar,
    // virtmcu-allow: mutex reasoning="Required for Condvar::wait_yielding_bql"
    wait_mutex: virtmcu_qom::sync::Mutex<()>,
    pub receiver: Option<virtmcu_qom::sync::VtimeIngress<SensorFrame>>,
    pub _liveliness: Option<Box<dyn virtmcu_wire::LivelinessToken>>,
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

impl virtmcu_qom::device::Peripheral for VirtmcuSensorState {
    fn read(
        &self,
        addr: u64,
        size: u32,
        _ctx: &virtmcu_qom::device::BqlContext,
    ) -> virtmcu_qom::device::MmioResult<'_> {
        virtmcu_qom::device::MmioDevice::read(self, addr, size)
    }

    fn write(&self, addr: u64, val: u64, size: u32, _ctx: &virtmcu_qom::device::BqlContext) {
        virtmcu_qom::device::MmioDevice::write(self, addr, val, size);
    }

    fn condvar(&self) -> &virtmcu_qom::sync::Condvar {
        virtmcu_qom::device::MmioDevice::condvar(self)
    }

    fn wait_mutex(&self) -> &virtmcu_qom::sync::Mutex<()> {
        virtmcu_qom::device::MmioDevice::wait_mutex(self)
    }
}

impl virtmcu_qom::device::MmioDevice for VirtmcuSensorState {
    fn read(&self, addr: u64, size: u32) -> virtmcu_qom::device::MmioResult<'_> {
        let s = unsafe { &mut *(self.parent_ptr as *mut VirtmcuSensorQEMU) }; // virtmcu-allow: unsafe_in_peripheral reasoning="Migration debt"
                                                                              // virtmcu-allow: new_unchecked_in_peripheral reasoning="Migration debt"
        let binding = unsafe { virtmcu_qom::device::BqlContext::new_unchecked() }; // virtmcu-allow: unsafe_in_peripheral reasoning="Migration debt"
        let inner = self.inner.get_mut(&binding);
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
                        // virtmcu-allow: new_unchecked_in_peripheral reasoning="Migration debt"
                        let binding = unsafe { virtmcu_qom::device::BqlContext::new_unchecked() }; // virtmcu-allow: unsafe_in_peripheral reasoning="Migration debt"
                        let i2 = self.inner.get(&binding);
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
                    || 0,
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
        let s = unsafe { &mut *(self.parent_ptr as *mut VirtmcuSensorQEMU) }; // virtmcu-allow: unsafe_in_peripheral reasoning="Migration debt"
                                                                              // virtmcu-allow: new_unchecked_in_peripheral reasoning="Migration debt"
        let binding = unsafe { virtmcu_qom::device::BqlContext::new_unchecked() }; // virtmcu-allow: unsafe_in_peripheral reasoning="Migration debt"
        let mut inner = self.inner.get_mut(&binding);
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
    #[rustfmt::skip]
    fn wait_mutex(&self) -> &virtmcu_qom::sync::Mutex<()> { // virtmcu-allow: mutex reasoning="State managed securely"
        &self.wait_mutex
    }
}

/// # Safety
/// This function is called by QEMU.
fn decode_cb(
    _opaque: *mut core::ffi::c_void,
    topic_str: &str,
    payload: &[u8],
) -> Option<SensorFrame> {
    if let Some((vtime, _seq, data_ptr)) = virtmcu_wire::decode_frame(payload) {
        if let Some(sensor_id_str) = topic_str.split('/').next_back() {
            let id_str = if sensor_id_str.starts_with("resd_") {
                sensor_id_str.rsplit_once('_').map_or(sensor_id_str, |(_, id)| id)
            } else {
                sensor_id_str.strip_prefix("sensordata_").unwrap_or(sensor_id_str)
            };

            if let Ok(sensor_id) = id_str.parse::<u32>() {
                let mut data = [0.0f64; MAX_DATA_ELEMENTS];
                let mut count = 0;
                for (d, chunk) in data.iter_mut().zip(data_ptr.chunks_exact(F64_SIZE_BYTES_USIZE)) {
                    *d = f64::from_le_bytes(chunk.try_into().expect("Invalid data format"));
                    count += 1;
                }
                let data_size = count as u32;

                return Some(SensorFrame { delivery_vtime_ns: vtime, sensor_id, data, data_size });
            }
        }
    }
    None
}

fn deliver_cb(opaque: *mut core::ffi::c_void, frame: SensorFrame) {
    let state = unsafe { &mut *(opaque as *mut VirtmcuSensorState) }; // virtmcu-allow: unsafe_in_peripheral reasoning="Migration debt"
                                                                      // virtmcu-allow: new_unchecked_in_peripheral reasoning="Migration debt"
    let binding = unsafe { virtmcu_qom::device::BqlContext::new_unchecked() }; // virtmcu-allow: unsafe_in_peripheral reasoning="Migration debt"
    let mut inner = state.inner.get_mut(&binding);
    if !inner.running {
        return;
    }

    virtmcu_qom::vlog!(
        "sensor_rx_timer_cb: processing frame for vtime {}\n",
        frame.delivery_vtime_ns
    );
    inner.map.insert(
        frame.sensor_id,
        SensorEntry { data: frame.data, data_size: frame.data_size, new_data: true },
    );

    let _guard = state.wait_mutex.lock();
    state.cond.notify_all();
    virtmcu_qom::vlog!("sensor_rx_timer_cb: notified!\n");
}

fn sensor_init_internal(
    s: &VirtmcuSensorQEMU,
    node_id: u32,
    transport: alloc::sync::Arc<dyn virtmcu_wire::DataTransport>,
) -> *mut VirtmcuSensorState {
    let mut state_box = Box::new(VirtmcuSensorState {
        parent_ptr: core::ptr::from_ref::<VirtmcuSensorQEMU>(s).cast_mut(),
        inner: BqlGuarded::new(VirtmcuSensorInner { map: HashMap::new(), running: true }),
        drain: virtmcu_qom::sync::VcpuDrain::new(),
        cond: virtmcu_qom::sync::Condvar::new(),
        wait_mutex: virtmcu_qom::sync::Mutex::new(()),
        receiver: None,
        _liveliness: None,
    });

    let state_ptr = core::ptr::from_mut(&mut *state_box);

    let node_id_str = node_id.to_string();
    let topic = sim_topic::sensor_data_wildcard(&node_id_str);

    let generation = alloc::sync::Arc::new(core::sync::atomic::AtomicU64::new(0));

    let receiver = virtmcu_qom::sync::VtimeIngress::new(
        transport.as_ref(),
        &topic,
        generation,
        state_ptr as *mut core::ffi::c_void,
        decode_cb,
        deliver_cb,
    );

    match receiver {
        Ok(r) => state_box.receiver = Some(r),
        Err(e) => {
            virtmcu_qom::sim_err!("Failed to subscribe to topic {}: {}", topic, e);
            return ptr::null_mut();
        }
    }

    let hb_topic = format!("sim/sensor/liveliness/{node_id}");
    let liveliness = transport.declare_liveliness(&hb_topic);

    state_box._liveliness = liveliness;

    Box::into_raw(state_box)
}

impl Drop for VirtmcuSensorState {
    fn drop(&mut self) {
        // virtmcu-allow: new_unchecked_in_peripheral reasoning="Migration debt"
        let binding = unsafe { virtmcu_qom::device::BqlContext::new_unchecked() }; // virtmcu-allow: unsafe_in_peripheral reasoning="Migration debt"
        self.inner.get_mut(&binding).running = false;
        self.cond.notify_all();
        self.drain.wait_for_drain(DEFAULT_TIMEOUT_MS);
    }
}

virtmcu_qom::define_properties!(
    VIRTMCU_SENSOR_PROPERTIES,
    [
        define_prop_uint32!(c"node".as_ptr(), VirtmcuSensorQEMU, node_id, 0),
        define_prop_string!(c"topic-prefix".as_ptr(), VirtmcuSensorQEMU, topic_prefix),
        define_prop_bool!(c"debug".as_ptr(), VirtmcuSensorQEMU, debug, false),
    ]
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sensor_layout() {
        assert_eq!(
            core::mem::offset_of!(VirtmcuSensorQEMU, parent_obj),
            0,
            "SysBusDevice must be the first field"
        );
    }
}
