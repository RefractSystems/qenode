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
//! This file is the "Gold Standard" implementation mandated by RFC-0021.
//! ============================================================================

extern crate alloc;

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use virtmcu_api::DataTransport;
use virtmcu_qom::memory::MemoryRegion;
use virtmcu_qom::qdev::SysBusDevice;
use virtmcu_qom::sync::{Condvar, DeliveryPacket, DeterministicReceiver, Mutex, VcpuDrain};

const REFERENCE_REG_STATUS: u64 = 0x00;
const REFERENCE_REG_TX: u64 = 0x04;
const REFERENCE_REG_8: u64 = 0x08;
const REFERENCE_VAL_FACEBABE: u64 = 0xface_babe;

/// A custom packet structure representing incoming data.
#[derive(Eq, PartialEq)]
pub struct ReferencePacket {
    pub vtime: u64,
    pub data: alloc::vec::Vec<u8>,
}

impl PartialOrd for ReferencePacket {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ReferencePacket {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.vtime.cmp(&other.vtime)
    }
}

impl DeliveryPacket for ReferencePacket {
    fn delivery_vtime_ns(&self) -> u64 {
        self.vtime
    }
}

/// The QEMU C-FFI Boundary Object.
/// (RFC-0023 Phase 4: Zero Unsafe Boilerplate)
#[repr(C)]
#[derive(virtmcu_qom::MmioDevice)]
#[virtmcu_qom::macros::qom_device(name = "reference-peripheral")]
pub struct ReferencePeripheralQEMU {
    pub parent_obj: SysBusDevice,
    pub iomem: MemoryRegion,

    #[qom_property]
    pub base_addr: u64,
    #[qom_property]
    pub debug: bool,
    #[qom_property]
    pub node_id: u32,
    #[qom_property]
    pub topic: virtmcu_qom::qom::QomString,

    #[qom_link(target = "virtmcu-transport-hub")]
    pub transport: virtmcu_qom::qom::QomLink<dyn DataTransport>,

    #[qom_state]
    pub state: ReferencePeripheralState,
}

pub struct ReferencePeripheralState {
    pub debug: bool,
    pub node_id: u32,
    pub topic: alloc::string::String,
    pub drain: VcpuDrain,
    pub cond: Arc<Condvar>,
    pub wait_mutex: Arc<Mutex<()>>,
    pub transport: Option<Arc<dyn DataTransport>>,
    pub receiver: Option<DeterministicReceiver<ReferencePacket>>,
    pub generation: Arc<AtomicU64>,
    pub has_data: Arc<AtomicBool>,
    pub tx_sequence: AtomicU64,
}

impl virtmcu_qom::device::PeripheralState for ReferencePeripheralState {
    type QomType = ReferencePeripheralQEMU;

    fn new(qemu_dev: &Self::QomType) -> Self {
        virtmcu_qom::sim_info!(
            ">>> HELLO FROM REFERENCE PERIPHERAL NEW! Node {}",
            qemu_dev.node_id
        );
        let topic = if qemu_dev.topic.is_null() {
            alloc::string::String::from("reference")
        } else {
            qemu_dev.topic.as_string()
        };

        Self {
            debug: qemu_dev.debug,
            node_id: qemu_dev.node_id,
            topic,
            drain: VcpuDrain::new(),
            cond: Arc::new(Condvar::new()),
            wait_mutex: Arc::new(Mutex::new(())),
            transport: qemu_dev.transport.get(),
            receiver: None,
            generation: Arc::new(AtomicU64::new(0)),
            has_data: Arc::new(AtomicBool::new(false)),
            tx_sequence: AtomicU64::new(0),
        }
    }
}

impl virtmcu_qom::device::Peripheral for ReferencePeripheralState {
    fn realize(&mut self) -> Result<(), alloc::string::String> {
        // Initialize Deterministic Ingress (RFC-0023 Step 1 & 4)
        if let Some(t) = &self.transport {
            let rx_topic = alloc::format!("sim/chardev/{}/rx", self.node_id);
            let generation_clone = Arc::clone(&self.generation);
            let has_data_clone = Arc::clone(&self.has_data);
            let cond_clone = Arc::clone(&self.cond);

            let rec = DeterministicReceiver::new_safe(
                &**t,
                &rx_topic,
                generation_clone,
                |_topic, payload| {
                    if let Some((header, data)) = virtmcu_api::decode_frame(payload) {
                        Some(ReferencePacket {
                            vtime: header.delivery_vtime_ns(),
                            data: data.to_vec(),
                        })
                    } else {
                        None
                    }
                },
                move |_packet| {
                    has_data_clone.store(true, Ordering::Release);
                    cond_clone.notify_all();
                },
            )
            .map_err(|e| alloc::format!("Failed to init receiver: {e}"))?;

            self.receiver = Some(rec);
            virtmcu_qom::sim_debug!("Reference: Node {} initialized with transport", self.node_id);
        } else {
            virtmcu_qom::sim_debug!(
                "Reference: Node {} initialized without transport (standalone mode)",
                self.node_id
            );
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

const REFERENCE_TX_SIZE: usize = 8;

impl virtmcu_qom::device::MmioDevice for ReferencePeripheralState {
    fn read(&self, addr: u64, _size: u32) -> virtmcu_qom::device::MmioResult<'_> {
        let _guard = self.drain.acquire();
        match addr {
            REFERENCE_REG_STATUS => {
                virtmcu_qom::sim_debug!(
                    "Reference: Read STATUS (has_data={})",
                    self.has_data.load(Ordering::Acquire)
                );
                let has_data_clone = Arc::clone(&self.has_data);
                virtmcu_qom::device::MmioResult::wait_for(
                    move || has_data_clone.load(Ordering::Acquire),
                    || 1,
                    || 0,
                )
            }
            REFERENCE_REG_8 => virtmcu_qom::device::MmioResult::Ready(REFERENCE_VAL_FACEBABE),
            _ => virtmcu_qom::device::MmioResult::Ready(0),
        }
    }

    fn write(&self, addr: u64, val: u64, _size: u32) {
        let _guard = self.drain.acquire();
        virtmcu_qom::sim_debug!(
            "[DEBUG] ReferencePeripheralState::write 0x{:x} = 0x{:x}",
            addr,
            val
        );
        if addr == REFERENCE_REG_TX {
            if let Some(transport) = &self.transport {
                let tx_topic = alloc::format!("sim/chardev/{}/tx", self.node_id);
                let vtime = virtmcu_qom::telemetry::get_global_vtime();
                let seq = self.tx_sequence.fetch_add(1, Ordering::SeqCst);

                virtmcu_qom::sim_info!(
                    "Reference: Committing packet to {} at vtime {}",
                    tx_topic,
                    vtime
                );

                // Zero-Copy Reservation API (RFC-0025) - Gold Standard Pattern
                match transport.reserve(&tx_topic, REFERENCE_TX_SIZE) {
                    Ok(mut reservation) => {
                        reservation.buffer_mut().copy_from_slice(&val.to_le_bytes());
                        let _ = reservation.commit(vtime, seq);
                    }
                    Err(e) => {
                        panic!(
                            "FATAL: Reference failed to reserve transport for topic {tx_topic}: {e:?}",
                        );
                    }
                };
            } else {
                virtmcu_qom::sim_info!("Reference: Write to TX but NO transport!");
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
virtmcu_qom::register_peripheral!(ReferencePeripheralQEMU);

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn test_reference_peripheral_qemu_layout() {
        assert_eq!(core::mem::offset_of!(ReferencePeripheralQEMU, parent_obj), 0);
    }

    #[test]
    fn test_reference_packet_ordering() {
        let p1 = ReferencePacket { vtime: 100, data: vec![1] };
        let p2 = ReferencePacket { vtime: 200, data: vec![2] };
        let p3 = ReferencePacket { vtime: 100, data: vec![3] };

        assert!(p1 < p2);
        assert!(p2 > p1);
        // Packets with the same vtime should be considered equal for ordering purposes
        assert_eq!(p1.cmp(&p3), core::cmp::Ordering::Equal);
    }

    #[test]
    fn test_reference_peripheral_state_logic() {
        // We can test basic state isolation without full QEMU realization
        let mut qemu_dev = ReferencePeripheralQEMU::new_mock();
        qemu_dev.base_addr = 0x1000;
        qemu_dev.debug = false;
        qemu_dev.node_id = 1;
        qemu_dev.topic = virtmcu_qom::qom::QomString::default();
        // transport defaults to empty mock QomLink through new_mock() zeroing

        let state =
            <ReferencePeripheralState as virtmcu_qom::device::PeripheralState>::new(&qemu_dev);

        assert_eq!(state.node_id, 1);
        assert_eq!(state.topic, "reference");
        assert!(!state.has_data.load(Ordering::SeqCst));

        // Simulate data arrival
        state.has_data.store(true, Ordering::SeqCst);
        assert!(state.has_data.load(Ordering::SeqCst));
    }
}
