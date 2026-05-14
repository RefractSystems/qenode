#![allow(clippy::panic)] // virtmcu-allow: allow reasoning="Fail Loudly"
#![allow(clippy::if_not_else)]
#![cfg_attr(
    test,
    allow(
        clippy::expect_used,
        clippy::unwrap_used,
        clippy::indexing_slicing,
        clippy::panic_in_result_fn
    )
)]
// std is required: virtmcu-qom dependency brings in std
//! ============================================================================
//! Welcome to the VirtMCU Peripheral Template!
//!
//! This file is the "Gold Standard" implementation mandated by ADR-021.
//! ============================================================================

extern crate alloc;

use alloc::sync::Arc;
use core::ffi::{c_char, c_void};
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use virtmcu_api::DataTransport;
use virtmcu_qom::memory::MemoryRegion;
use virtmcu_qom::qdev::SysBusDevice;
use virtmcu_qom::sync::{Condvar, DeliveryPacket, DeterministicReceiver, Mutex, VcpuDrain};

const DUMMY_REG_STATUS: u64 = 0x00;
const DUMMY_REG_TX: u64 = 0x04;
const DUMMY_REG_8: u64 = 0x08;
const DUMMY_VAL_FACEBABE: u64 = 0xface_babe;

/// A custom packet structure representing incoming data.
#[derive(Eq, PartialEq)]
pub struct DummyPacket {
    pub vtime: u64,
    pub data: alloc::vec::Vec<u8>,
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
    fn delivery_vtime_ns(&self) -> u64 {
        self.vtime
    }
}

/// The QEMU C-FFI Boundary Object.
/// (RFC-0023 Phase 4: Zero Unsafe Boilerplate)
#[repr(C)]
#[derive(virtmcu_qom::MmioDevice)]
#[virtmcu_qom::macros::qom_device(name = "rust-dummy")]
pub struct RustDummyQEMU {
    pub parent_obj: SysBusDevice,
    pub iomem: MemoryRegion,

    #[qom_property]
    pub base_addr: u64,
    #[qom_property]
    pub debug: bool,
    #[qom_property]
    pub node_id: u32,
    #[qom_property]
    pub topic: *mut c_char,

    #[qom_link(target = "virtmcu-transport-hub")]
    pub transport: virtmcu_qom::qom::QomLink<dyn DataTransport>,

    #[qom_state]
    pub state: RustDummyState,
}

pub struct RustDummyState {
    pub debug: bool,
    pub node_id: u32,
    pub topic: alloc::string::String,
    pub drain: VcpuDrain,
    pub cond: Condvar,
    pub wait_mutex: Mutex<()>,
    pub transport: Option<Arc<dyn DataTransport>>,
    pub receiver: Option<DeterministicReceiver<DummyPacket>>,
    pub generation: Arc<AtomicU64>,
    pub has_data: AtomicBool,
    pub qemu_dev_ptr: *mut RustDummyQEMU,
}

impl virtmcu_qom::device::PeripheralState for RustDummyState {
    type QomType = RustDummyQEMU;

    fn new(qemu_dev: &Self::QomType) -> Self {
        let topic = if qemu_dev.topic.is_null() {
            alloc::string::String::from("dummy")
        } else {
            let mut len = 0;
            while unsafe { *qemu_dev.topic.add(len) } != 0 {
                len += 1;
            }
            let slice = unsafe { core::slice::from_raw_parts(qemu_dev.topic.cast::<u8>(), len) };
            alloc::string::String::from_utf8_lossy(slice).into_owned()
        };

        Self {
            debug: qemu_dev.debug,
            node_id: qemu_dev.node_id,
            topic,
            drain: VcpuDrain::new(),
            cond: Condvar::new(),
            wait_mutex: Mutex::new(()),
            transport: qemu_dev.transport.get(),
            receiver: None,
            generation: Arc::new(AtomicU64::new(0)),
            has_data: AtomicBool::new(false),
            qemu_dev_ptr: core::ptr::from_ref(qemu_dev).cast_mut(),
        }
    }
}

impl virtmcu_qom::device::Peripheral for RustDummyState {
    fn realize(&mut self) -> Result<(), alloc::string::String> {
        // Initialize Deterministic Ingress (RFC-0023 Step 1 & 4)
        if let Some(t) = &self.transport {
            let rx_topic = alloc::format!("sim/chardev/{}/rx", self.node_id);
            let generation_clone = Arc::clone(&self.generation);

            let rec = DeterministicReceiver::new(
                &**t,
                &rx_topic,
                generation_clone,
                self.qemu_dev_ptr as *mut c_void,
                |_opaque, _topic, payload| {
                    if let Some((header, data)) = virtmcu_api::decode_frame(payload) {
                        Some(DummyPacket { vtime: header.delivery_vtime_ns(), data: data.to_vec() })
                    } else {
                        None
                    }
                },
                |opaque, _packet| {
                    let dev = unsafe { &mut *(opaque as *mut RustDummyQEMU) };
                    if !dev.state.is_null() {
                        let state = unsafe { &*dev.state };
                        state.has_data.store(true, Ordering::Release);
                    }
                },
            )
            .map_err(|e| alloc::format!("Failed to init receiver: {e}"))?;

            self.receiver = Some(rec);
        }
        Ok(())
    }

    fn read(
        &self,
        addr: u64,
        size: u32,
        _token: &virtmcu_qom::device::DrainToken,
    ) -> virtmcu_qom::device::MmioResult<'_> {
        virtmcu_qom::device::MmioDevice::read(self, addr, size)
    }

    fn write(&self, addr: u64, val: u64, size: u32, _token: &virtmcu_qom::device::DrainToken) {
        virtmcu_qom::device::MmioDevice::write(self, addr, val, size);
    }

    fn condvar(&self) -> &Condvar {
        &self.cond
    }

    fn wait_mutex(&self) -> &Mutex<()> {
        &self.wait_mutex
    }
}

const DUMMY_TX_SIZE: usize = 8;

impl virtmcu_qom::device::MmioDevice for RustDummyState {
    fn read(&self, addr: u64, _size: u32) -> virtmcu_qom::device::MmioResult<'_> {
        let _guard = self.drain.acquire();
        match addr {
            DUMMY_REG_STATUS => virtmcu_qom::device::MmioResult::wait_for(
                || self.has_data.load(Ordering::Acquire),
                || 1,
                || 0,
            ),
            DUMMY_REG_8 => virtmcu_qom::device::MmioResult::Ready(DUMMY_VAL_FACEBABE),
            _ => virtmcu_qom::device::MmioResult::Ready(0),
        }
    }

    fn write(&self, addr: u64, val: u64, _size: u32) {
        let _guard = self.drain.acquire();
        if addr == DUMMY_REG_TX {
            if let Some(transport) = &self.transport {
                let tx_topic = alloc::format!("sim/chardev/{}/tx", self.node_id);
                // Zero-Copy Reservation API (RFC-0025) - Gold Standard Pattern
                match transport.reserve(&tx_topic, DUMMY_TX_SIZE) {
                    Ok(mut reservation) => {
                        reservation.buffer_mut().copy_from_slice(&val.to_le_bytes());
                        let _ = reservation.commit(0, 0);
                    }
                    Err(e) => {
                        if self.debug {
                            virtmcu_qom::sim_err!(
                                "Dummy: Failed to reserve transport for topic {}: {:?}",
                                tx_topic,
                                e
                            );
                        }
                    }
                };
            }
        }
    }

    fn condvar(&self) -> &Condvar {
        &self.cond
    }

    fn wait_mutex(&self) -> &Mutex<()> {
        &self.wait_mutex
    }
}

// RFC-0023 Phase 5: DSO Registration
virtmcu_qom::register_peripheral!(RustDummyQEMU);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_rust_dummy_qemu_layout() {
        assert_eq!(core::mem::offset_of!(RustDummyQEMU, parent_obj), 0);
    }
}
