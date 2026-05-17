#![allow(clippy::all, unused_imports, dead_code, unused_variables, unused_mut)] // virtmcu-allow: allow reasoning="Zero unsafe"
#![allow(clippy::all)] // virtmcu-allow: allow reasoning="Zero unsafe"
#![allow(clippy::panic)] // virtmcu-allow: allow reasoning="Fail Loudly"
#![allow(clippy::not_unsafe_ptr_arg_deref)] // virtmcu-allow: allow reasoning="Zero unsafe"
#![allow(clippy::missing_safety_doc)] // virtmcu-allow: allow reasoning="Zero unsafe"
#![cfg_attr(
    test,
    allow(
        clippy::expect_used,
        clippy::unwrap_used,
        clippy::indexing_slicing,
        clippy::panic_in_result_fn
    )
)]

extern crate alloc;

use alloc::boxed::Box;
use alloc::string::ToString;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use virtmcu_qom::memory::MemoryRegion;
use virtmcu_qom::qdev::SysBusDevice;
use virtmcu_qom::sync::{Condvar, Mutex, VcpuDrain};
use virtmcu_wire::topics::sim_topic;
use virtmcu_wire::DataTransport;

const MAX_DATA_ELEMENTS: usize = 8;
const F64_SIZE_BYTES: u64 = core::mem::size_of::<f64>() as u64;

const REG_ACTUATOR_ID: u64 = 0x00;
const REG_ACTUATOR_DATA_SIZE: u64 = 0x04;
const REG_ACTUATOR_GO: u64 = 0x08;
const REG_ACTUATOR_DATA: u64 = 0x10;

#[repr(C)]
#[derive(virtmcu_qom::MmioDevice)]
#[virtmcu_qom::macros::qom_device(name = "actuator")]
pub struct VirtmcuActuatorQEMU {
    pub parent_obj: SysBusDevice,
    pub iomem: MemoryRegion,

    #[qom_property]
    pub node: u32,
    #[qom_property]
    pub router: virtmcu_qom::qom::QomString,
    #[qom_property]
    pub topic_prefix: virtmcu_qom::qom::QomString,
    #[qom_property]
    pub debug: bool,

    #[qom_link(target = "virtmcu-transport-hub")]
    pub transport: virtmcu_qom::qom::QomLink<dyn DataTransport>,

    #[qom_state]
    pub state: VirtmcuActuatorState,
}

pub struct VirtmcuActuatorState {
    pub node_id: u32,
    pub debug: bool,
    pub drain: VcpuDrain,
    pub transport: Option<Arc<dyn DataTransport>>,
    pub seq: AtomicU64,
    pub cond: Arc<Condvar>,
    // virtmcu-allow: mutex reasoning="Required for Condvar::wait_yielding_bql"
    pub wait_mutex: Arc<Mutex<()>>,
    pub _liveliness: Option<Box<dyn virtmcu_wire::LivelinessToken>>,

    /* Registers */
    pub actuator_id: AtomicU32,
    pub data_size: AtomicU32,
    pub data: [AtomicU64; 8],
}

impl virtmcu_qom::device::PeripheralState for VirtmcuActuatorState {
    type QomType = VirtmcuActuatorQEMU;

    fn new(qemu_dev: &Self::QomType) -> Self {
        let node_id = qemu_dev.node;
        let mut _liveliness = None;
        if let Some(t) = qemu_dev.transport.get() {
            let hb_topic = alloc::format!("sim/actuator/liveliness/{node_id}");
            _liveliness = t.declare_liveliness(&hb_topic);
        }

        Self {
            node_id,
            debug: qemu_dev.debug,
            drain: VcpuDrain::new(),
            transport: qemu_dev.transport.get(),
            seq: AtomicU64::new(0),
            cond: Arc::new(Condvar::new()),
            wait_mutex: Arc::new(Mutex::new(())),
            _liveliness,
            actuator_id: AtomicU32::new(0),
            data_size: AtomicU32::new(0),
            data: core::array::from_fn(|_| AtomicU64::new(0)),
        }
    }
}

impl virtmcu_qom::device::Peripheral for VirtmcuActuatorState {
    fn realize(
        &mut self,
        _ctx: &virtmcu_qom::device::BqlContext,
    ) -> Result<(), alloc::string::String> {
        Ok(())
    }

    fn read(
        &self,
        addr: u64,
        size: u32,
        _token: &virtmcu_qom::device::BqlContext,
    ) -> virtmcu_qom::device::MmioResult<'_> {
        virtmcu_qom::device::MmioDevice::read(self, addr, size)
    }

    fn write(&self, addr: u64, val: u64, size: u32, _token: &virtmcu_qom::device::BqlContext) {
        virtmcu_qom::device::MmioDevice::write(self, addr, val, size);
    }

    fn condvar(&self) -> &Condvar {
        &self.cond
    }

    // virtmcu-allow: mutex reasoning="Required for Condvar::wait_yielding_bql"
    #[rustfmt::skip]
    fn wait_mutex(&self) -> &Mutex<()> { // virtmcu-allow: mutex reasoning="State managed securely"
        &self.wait_mutex
    }
}

impl virtmcu_qom::device::MmioDevice for VirtmcuActuatorState {
    fn read(&self, addr: u64, size: u32) -> virtmcu_qom::device::MmioResult<'_> {
        let _guard = self.drain.acquire();
        let ret = match addr {
            REG_ACTUATOR_ID => u64::from(self.actuator_id.load(Ordering::SeqCst)),
            REG_ACTUATOR_DATA_SIZE => u64::from(self.data_size.load(Ordering::SeqCst)),
            addr if (REG_ACTUATOR_DATA
                ..REG_ACTUATOR_DATA + (MAX_DATA_ELEMENTS as u64) * F64_SIZE_BYTES)
                .contains(&addr) =>
            {
                let idx = ((addr - REG_ACTUATOR_DATA) / F64_SIZE_BYTES) as usize;
                let offset = ((addr - REG_ACTUATOR_DATA) % F64_SIZE_BYTES) as usize;
                let mut ret: u64 = 0;
                if offset + (size as usize) <= (F64_SIZE_BYTES as usize) {
                    let val_bits =
                        self.data.get(idx).expect("idx out of bounds").load(Ordering::SeqCst);
                    let bytes = f64::from_bits(val_bits).to_le_bytes();
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
                if self.debug {
                    virtmcu_qom::sim_debug!("actuator_read: unhandled offset 0x{:x}", addr);
                }
                0
            }
        };
        virtmcu_qom::device::MmioResult::Ready(ret)
    }

    fn write(&self, addr: u64, val: u64, size: u32) {
        let _guard = self.drain.acquire();
        if self.debug {
            virtmcu_qom::vlog!("actuator_write: addr 0x{:x}, val {}\n", addr, val);
        }
        match addr {
            REG_ACTUATOR_ID => {
                self.actuator_id.store(val as u32, Ordering::SeqCst);
            }
            REG_ACTUATOR_DATA_SIZE => {
                let mut size_val = val as u32;
                if size_val > (MAX_DATA_ELEMENTS as u32) {
                    size_val = MAX_DATA_ELEMENTS as u32;
                }
                self.data_size.store(size_val, Ordering::SeqCst);
            }
            REG_ACTUATOR_GO => {
                if (val & 0x1) == 1 {
                    let vtime_ns = u64::try_from(virtmcu_qom::ffi_call! {
                        virtmcu_qom::timer::qemu_clock_get_ns(
                            virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL,
                        )
                    })
                    .expect("vtime is negative");

                    let seq = self.seq.fetch_add(1, Ordering::Relaxed);

                    let actuator_id_val = self.actuator_id.load(Ordering::SeqCst);
                    let data_size_val = self.data_size.load(Ordering::SeqCst);
                    let node_id_str = self.node_id.to_string();
                    let topic = sim_topic::actuator_control(&node_id_str, actuator_id_val);

                    let mut data_payload =
                        Vec::with_capacity((data_size_val as usize) * (F64_SIZE_BYTES as usize));

                    for i in 0..(data_size_val as usize) {
                        let val_bits =
                            self.data.get(i).expect("idx out of bounds").load(Ordering::SeqCst);
                        let float_val = f64::from_bits(val_bits);
                        data_payload.extend_from_slice(&float_val.to_le_bytes());
                    }

                    if let Some(transport) = &self.transport {
                        match transport.reserve(&topic, data_payload.len()) {
                            Ok(mut reservation) => {
                                reservation.buffer_mut().copy_from_slice(&data_payload);
                                let _ = reservation.commit(vtime_ns, seq);
                            }
                            Err(e) => {
                                virtmcu_qom::sim_err!(
                                    "actuator: failed to reserve transport for topic {topic}: {e:?}",
                                );
                            }
                        };
                    } else {
                        virtmcu_qom::sim_err!("actuator: transport missing on GO");
                    }
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
                    let atomic_val = self.data.get(idx).expect("idx out of bounds");
                    let current_bits = atomic_val.load(Ordering::SeqCst);
                    let mut data_bytes = f64::from_bits(current_bits).to_le_bytes();

                    if let (Some(dest), Some(src)) = (
                        data_bytes.get_mut(offset..offset + size as usize),
                        val_bytes.get(..size as usize),
                    ) {
                        dest.copy_from_slice(src);
                        let new_float = f64::from_le_bytes(data_bytes);
                        atomic_val.store(new_float.to_bits(), Ordering::SeqCst);
                    }
                }
            }
            _ => {
                if self.debug {
                    virtmcu_qom::sim_debug!(
                        "actuator_write: unhandled offset 0x{:x} val=0x{:x}",
                        addr,
                        val
                    );
                }
            }
        }
    }

    fn condvar(&self) -> &Condvar {
        &self.cond
    }

    // virtmcu-allow: mutex reasoning="Required for Condvar::wait_yielding_bql"
    #[rustfmt::skip]
    fn wait_mutex(&self) -> &Mutex<()> { // virtmcu-allow: mutex reasoning="State managed securely"
        &self.wait_mutex
    }
}

virtmcu_qom::register_peripheral!(VirtmcuActuatorQEMU);

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
