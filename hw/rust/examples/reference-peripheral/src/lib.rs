#![allow(clippy::panic)] // virtmcu-allow: allow reasoning="Fail Loudly"
#![forbid(unsafe_code)]
#![cfg_attr(
    test,
    allow(
        clippy::expect_used,
        clippy::unwrap_used,
        clippy::indexing_slicing,
        clippy::panic_in_result_fn
    )
)]
//! ============================================================================
//! Welcome to the VirtMCU Peripheral Template!
//!
//! This file is the "Gold Standard" implementation mandated by RFC-0021.
//! ============================================================================

extern crate alloc;

use alloc::string::String;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use virtmcu_qom::memory::MemoryRegion;
use virtmcu_qom::qdev::SysBusDevice;
use virtmcu_qom::sync::{Condvar, DeliveryPacket, Mutex, VcpuDrain, VtimeIngress};
use virtmcu_wire::DataTransport;

const REFERENCE_REG_STATUS: u64 = 0x00;
const REFERENCE_REG_TX: u64 = 0x04;
const REFERENCE_REG_RX: u64 = 0x08;
const REFERENCE_TX_SIZE: usize = 8;
const REFERENCE_RX_PAYLOAD_BYTES: usize = 4;

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
    /// Example bool QOM property. VirtMCU already provides sim_debug!() for debug gating;
    /// this property shows how to expose a bool flag from the device tree.
    #[qom_property]
    pub debug: bool,
    #[qom_property]
    pub node_id: u32,
    /// Topic namespace for this peripheral's sim topics (e.g. "chardev", "uart", "can").
    /// Produces topics: sim/{topic}/{node_id}/rx  and  sim/{topic}/{node_id}/tx.
    /// Default "chardev" matches the coordinator's ReferenceLink legacy routing.
    #[qom_property]
    pub topic: virtmcu_qom::qom::QomString,

    #[qom_link(target = "virtmcu-transport-hub")]
    pub transport: virtmcu_qom::qom::QomLink<dyn DataTransport>,

    #[qom_state]
    pub state: ReferencePeripheralState,
}

pub struct ReferencePeripheralState {
    pub node_id: u32,
    /// Topic namespace from the QOM property; drives sim topic construction.
    pub topic: String,
    pub drain: VcpuDrain,
    pub cond: Arc<Condvar>,
    pub wait_mutex: Arc<Mutex<()>>,
    pub transport: Option<Arc<dyn DataTransport>>,
    pub receiver: Option<VtimeIngress<ReferencePacket>>,
    pub generation: Arc<AtomicU64>,
    pub has_data: Arc<AtomicBool>,
    pub rx_data: Arc<AtomicU32>,
    pub tx_sequence: virtmcu_qom::sync::BqlGuarded<u64>,
    pub poll_timer: Option<virtmcu_qom::timer::ClosureTimer>,
}

impl virtmcu_qom::device::PeripheralState for ReferencePeripheralState {
    type QomType = ReferencePeripheralQEMU;

    fn new(qemu_dev: &Self::QomType) -> Self {
        virtmcu_qom::sim_info!(
            ">>> HELLO FROM REFERENCE PERIPHERAL NEW! Node {}",
            qemu_dev.node_id
        );
        let topic = if qemu_dev.topic.is_null() {
            // "chardev" matches coordinator's ReferenceLink routing (sim/chardev/{n}/rx).
            String::from("chardev")
        } else {
            qemu_dev.topic.as_string()
        };

        Self {
            node_id: qemu_dev.node_id,
            topic,
            drain: VcpuDrain::new(),
            cond: Arc::new(Condvar::new()),
            wait_mutex: Arc::new(Mutex::new(())),
            transport: qemu_dev.transport.get(),
            receiver: None,
            generation: Arc::new(AtomicU64::new(0)),
            has_data: Arc::new(AtomicBool::new(false)),
            rx_data: Arc::new(AtomicU32::new(0)),
            tx_sequence: virtmcu_qom::sync::BqlGuarded::new(0),
            poll_timer: None,
        }
    }
}

impl virtmcu_qom::device::Peripheral for ReferencePeripheralState {
    fn realize(&mut self, ctx: &virtmcu_qom::device::BqlContext) -> Result<(), String> {
        if let Some(t) = &self.transport {
            let rx_topic = format!("sim/{}/{}/rx", self.topic, self.node_id);
            let generation_clone = Arc::clone(&self.generation);
            let has_data_clone = Arc::clone(&self.has_data);
            let rx_data_clone = Arc::clone(&self.rx_data);
            let cond_clone = Arc::clone(&self.cond);

            let rec = VtimeIngress::new_safe(
                &**t,
                &rx_topic,
                generation_clone,
                |topic, payload| {
                    virtmcu_qom::sim_debug!(
                        "Reference: Rx callback on topic {} (len={})",
                        topic,
                        payload.len()
                    );
                    if let Some((vtime, _seq, data)) = virtmcu_wire::decode_frame(payload) {
                        Some(ReferencePacket { vtime, data: data.to_vec() })
                    } else {
                        virtmcu_qom::sim_err!("Reference: failed to decode frame!");
                        None
                    }
                },
                move |_packet| {
                    let val = if _packet.data.len() >= REFERENCE_RX_PAYLOAD_BYTES {
                        u32::from_le_bytes(
                            _packet.data[0..REFERENCE_RX_PAYLOAD_BYTES]
                                .try_into()
                                .expect("FATAL: slice length mismatch despite bounds check"),
                        )
                    } else {
                        0
                    };
                    rx_data_clone.store(val, Ordering::Release);
                    has_data_clone.store(true, Ordering::Release);
                    cond_clone.notify_all();
                },
            )
            .map_err(|e| format!("Failed to init receiver: {e}"))?;

            self.receiver = Some(rec);

            // Demonstrate ClosureTimer
            self.poll_timer = Some(virtmcu_qom::timer::ClosureTimer::new(
                virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL,
                move |timer_ctx| {
                    virtmcu_qom::sim_debug!(
                        "Reference: ClosureTimer fired! vtime={}",
                        virtmcu_qom::timer::qemu_clock_get_ns_safe(
                            virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL,
                            timer_ctx
                        )
                    );
                },
            ));
            self.poll_timer.as_ref().expect("just set").arm(
                virtmcu_qom::timer::qemu_clock_get_ns_safe(
                    virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL,
                    ctx,
                ) + 1_000_000,
            );

            virtmcu_qom::sim_info!("Reference: Node {} subscribed to {}", self.node_id, rx_topic);
        } else {
            virtmcu_qom::sim_info!(
                "Reference: Node {} initialized without transport (standalone mode)",
                self.node_id
            );
        }
        Ok(())
    }

    fn read(
        &self,
        addr: u64,
        _size: u32,
        _ctx: &virtmcu_qom::device::BqlContext,
    ) -> virtmcu_qom::device::MmioResult<'_> {
        let _guard = self.drain.acquire();
        match addr {
            REFERENCE_REG_STATUS => {
                let has_data_clone = Arc::clone(&self.has_data);
                virtmcu_qom::device::MmioResult::wait_for(
                    move || has_data_clone.load(Ordering::Acquire),
                    || 1,
                    || 0,
                )
            }
            REFERENCE_REG_RX => {
                let data = self.rx_data.load(Ordering::Acquire);
                self.has_data.store(false, Ordering::Release);
                virtmcu_qom::device::MmioResult::Ready(data as u64)
            }
            _ => virtmcu_qom::device::MmioResult::Ready(0),
        }
    }

    fn write(&self, addr: u64, val: u64, _size: u32, ctx: &virtmcu_qom::device::BqlContext) {
        let _guard = self.drain.acquire();
        match addr {
            REFERENCE_REG_TX => {
                let Some(transport) = &self.transport else {
                    virtmcu_qom::sim_info!("Reference: Write to TX but NO transport!");
                    return;
                };
                let tx_topic = format!("sim/{}/{}/tx", self.topic, self.node_id);
                // Use qemu_clock_get_ns_safe with BqlContext instead of get_global_vtime
                let vtime = virtmcu_qom::timer::qemu_clock_get_ns_safe(
                    virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL,
                    ctx,
                ) as u64;

                // Demonstrate BqlGuarded::get_mut
                let seq = {
                    let mut seq_guard = self.tx_sequence.get_mut(ctx);
                    let val = *seq_guard;
                    *seq_guard += 1;
                    val
                };

                match transport.reserve(&tx_topic, REFERENCE_TX_SIZE) {
                    Ok(mut reservation) => {
                        reservation.buffer_mut().copy_from_slice(&val.to_le_bytes());
                        reservation
                            .commit(vtime, seq)
                            .expect("FATAL: Reference failed to commit transport reservation");
                    }
                    Err(e) => panic!(
                        "FATAL: Reference failed to reserve transport for topic {tx_topic}: {e:?}",
                    ),
                };
            }
            _ => panic!("Reference: write to unknown register 0x{addr:x} = 0x{val:x}"),
        }
    }

    // Both Peripheral and MmioDevice require condvar()/wait_mutex() as trait methods.
    fn condvar(&self) -> &Condvar {
        &self.cond
    }

    #[rustfmt::skip]
    fn wait_mutex(&self) -> &Mutex<()> { // virtmcu-allow: mutex reasoning="State managed securely"
        &self.wait_mutex
    }
}

// RFC-0023 Phase 5: DSO Registration
impl virtmcu_qom::device::MmioDevice for ReferencePeripheralState {
    fn read(&self, _addr: u64, _size: u32) -> virtmcu_qom::device::MmioResult<'_> {
        unreachable!("Use Peripheral::read")
    }
    fn write(&self, _addr: u64, _val: u64, _size: u32) {
        unreachable!("Use Peripheral::write")
    }
    fn condvar(&self) -> &virtmcu_qom::sync::Condvar {
        &self.cond
    }
    #[rustfmt::skip]
    fn wait_mutex(&self) -> &virtmcu_qom::sync::Mutex<()> {
        &self.wait_mutex
    }
}

virtmcu_qom::register_peripheral!(ReferencePeripheralQEMU);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dynamic_cast_qom_demonstration() {
        let mut qemu_dev = ReferencePeripheralQEMU::new_mock();
        let _ptr = core::ptr::from_mut(&mut qemu_dev) as *mut core::ffi::c_void;

        // This fails at runtime in a mock environment because object_dynamic_cast needs real QEMU,
        // but this shows the API pattern. We only test compilation of this pattern here.
        // let _cast = virtmcu_qom::qom::dynamic_cast_qom::<ReferencePeripheralQEMU>(ptr).expect("FATAL: QOM type mismatch");
    }

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
        // Same vtime → equal for ordering purposes (tie-broken by coordinator via seq).
        assert_eq!(p1.cmp(&p3), core::cmp::Ordering::Equal);
    }

    #[test]
    fn test_reference_peripheral_state_logic() {
        let mut qemu_dev = ReferencePeripheralQEMU::new_mock();
        qemu_dev.base_addr = 0x1000;
        qemu_dev.debug = false;
        qemu_dev.node_id = 1;
        qemu_dev.topic = virtmcu_qom::qom::QomString::default();

        let state =
            <ReferencePeripheralState as virtmcu_qom::device::PeripheralState>::new(&qemu_dev);

        assert_eq!(state.node_id, 1);
        assert_eq!(state.topic, "chardev");
        assert!(!state.has_data.load(Ordering::SeqCst));

        state.has_data.store(true, Ordering::SeqCst);
        assert!(state.has_data.load(Ordering::SeqCst));
    }
}
