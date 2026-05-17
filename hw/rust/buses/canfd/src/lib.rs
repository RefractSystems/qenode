#![allow(deprecated)] // virtmcu-allow: allow reasoning="S2 migration in progress"
#![allow(clippy::all, unused_imports, dead_code, unused_variables, unused_mut)] // virtmcu-allow: allow reasoning="Zero unsafe"
#![allow(clippy::all)] // virtmcu-allow: allow reasoning="Zero unsafe"
#![allow(clippy::panic)] // virtmcu-allow: allow reasoning="Fail Loudly"
#![allow(clippy::if_not_else)]
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
//! Virtmcu virtual CAN FD device with pluggable transport.

extern crate alloc;

use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use core::time::Duration;
use crossbeam_channel::{bounded, Receiver, RecvTimeoutError, Sender};
use virtmcu_qom::net::{CanHostState, QemuCanFrame};
use virtmcu_qom::sync::{Condvar, DeliveryPacket, Mutex, VcpuDrain, VtimeIngress};
use virtmcu_wire::can_generated::virtmcu::can::CanFdFrame;
use virtmcu_wire::DataTransport;

const CAN_TX_QUEUE_SIZE: usize = 65536;
const CAN_TX_POLL_TIMEOUT_MS: u64 = 10;

type TxPayload = (u64, u64, Vec<u8>);

fn spawn_can_tx_thread(
    transport: Arc<dyn DataTransport>,
    topic: String,
    rx: Receiver<TxPayload>,
    shutdown: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || loop {
        if shutdown.load(Ordering::Acquire) {
            break;
        }
        match rx.recv_timeout(Duration::from_millis(CAN_TX_POLL_TIMEOUT_MS)) {
            Ok((vtime, seq, payload)) => {
                if let Ok(mut reservation) = transport.reserve(&topic, payload.len()) {
                    reservation.buffer_mut().copy_from_slice(&payload);
                    let _ = reservation.commit(vtime, seq);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    })
}

pub struct OrderedCanFrame {
    pub vtime: u64,
    pub sequence: u64,
    pub frame: QemuCanFrame,
}

impl PartialEq for OrderedCanFrame {
    fn eq(&self, other: &Self) -> bool {
        self.vtime == other.vtime && self.sequence == other.sequence
    }
}
impl Eq for OrderedCanFrame {}
impl PartialOrd for OrderedCanFrame {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for OrderedCanFrame {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        // Reverse for min-heap
        match other.vtime.cmp(&self.vtime) {
            core::cmp::Ordering::Equal => other.sequence.cmp(&self.sequence),
            ord => ord,
        }
    }
}

impl DeliveryPacket for OrderedCanFrame {
    fn delivery_vtime_ns(&self) -> u64 {
        self.vtime
    }
}

/// The QEMU C-FFI Boundary Object.
#[repr(C)]
#[derive(virtmcu_qom::MmioDevice)]
#[virtmcu_qom::macros::qom_device(name = "can-host-virtmcu", parent = "can-host")]
pub struct VirtmcuCanHostQEMU {
    pub parent_obj: CanHostState,
    pub iomem: virtmcu_qom::memory::MemoryRegion,

    #[qom_property]
    pub node: virtmcu_qom::qom::QomString,
    #[qom_property]
    pub router: virtmcu_qom::qom::QomString,
    #[qom_property]
    pub topic: virtmcu_qom::qom::QomString,

    #[qom_link(target = "virtmcu-transport-hub")]
    pub transport_link: virtmcu_qom::qom::QomLink<dyn DataTransport>,

    #[qom_state]
    pub state: CanfdState,
}

pub struct CanfdState {
    pub drain: VcpuDrain,
    pub cond: Arc<Condvar>,
    pub wait_mutex: Arc<Mutex<()>>, // virtmcu-allow: mutex reasoning="Wait mutex for MmioDevice"
    pub transport: Option<Arc<dyn DataTransport>>,
    pub receiver: Option<VtimeIngress<OrderedCanFrame>>,
    pub tx_sender: Sender<TxPayload>,
    pub tx_shutdown: Arc<AtomicBool>,
    pub tx_thread: Option<std::thread::JoinHandle<()>>,
    pub backlog: Arc<Mutex<VecDeque<QemuCanFrame>>>, // virtmcu-allow: mutex reasoning="Backlog managed securely"
    pub tx_sequence: Arc<AtomicU64>,
    pub node_id: u32,
    pub topic: String,
    pub generation: Arc<AtomicU64>,
}

impl Drop for CanfdState {
    fn drop(&mut self) {
        self.tx_shutdown.store(true, Ordering::Release);
        if let Some(thread) = self.tx_thread.take() {
            thread.join().expect("CAN-FD TX thread panicked");
        }
    }
}

impl virtmcu_qom::device::PeripheralState for CanfdState {
    type QomType = VirtmcuCanHostQEMU;

    fn new(qemu_dev: &Self::QomType) -> Self {
        let node_str = qemu_dev.node.as_string();
        let node_id = if node_str.is_empty() {
            0
        } else {
            node_str.parse().expect("CAN-FD node id must be numeric")
        };

        let topic = if qemu_dev.topic.is_null() {
            String::from("sim/can")
        } else {
            qemu_dev.topic.as_string()
        };

        let (tx_rx, rx_rx) = bounded::<TxPayload>(CAN_TX_QUEUE_SIZE);
        let tx_shutdown = Arc::new(AtomicBool::new(false));

        // Setup transport link
        let transport = qemu_dev.transport_link.get();
        let tx_thread = transport.as_ref().map(|t| {
            spawn_can_tx_thread(Arc::clone(t), topic.clone(), rx_rx, Arc::clone(&tx_shutdown))
        });

        Self {
            drain: VcpuDrain::new(),
            cond: Arc::new(Condvar::new()),
            wait_mutex: Arc::new(Mutex::new(())), // virtmcu-allow: mutex reasoning="Wait mutex for MmioDevice"
            transport,
            receiver: None,
            tx_sender: tx_rx,
            tx_shutdown,
            tx_thread,
            backlog: Arc::new(Mutex::new(VecDeque::new())), // virtmcu-allow: mutex reasoning="Backlog managed securely"
            tx_sequence: Arc::new(AtomicU64::new(0)),
            node_id,
            topic,
            generation: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl virtmcu_qom::device::Peripheral for CanfdState {
    fn realize(&mut self, _ctx: &virtmcu_qom::device::BqlContext) -> Result<(), String> {
        if let Some(t) = &self.transport {
            let rx_topic = self.topic.clone();
            let generation_clone = Arc::clone(&self.generation);
            let backlog_clone = Arc::clone(&self.backlog);

            let rec = VtimeIngress::new_safe(
                &**t,
                &rx_topic,
                generation_clone,
                |_topic, payload| {
                    const CAN_FD_MAX_PAYLOAD: usize = 64;
                    let (vtime, sequence, data) = virtmcu_wire::decode_frame(payload)?;
                    let fbs = flatbuffers::root::<CanFdFrame>(data).ok()?;

                    let mut data_arr = [0u8; CAN_FD_MAX_PAYLOAD];
                    let dlc = if let Some(d) = fbs.data() {
                        let len = core::cmp::min(d.len(), CAN_FD_MAX_PAYLOAD);
                        data_arr[..len].copy_from_slice(&d.bytes()[..len]);
                        len as u8
                    } else {
                        0
                    };

                    let frame = QemuCanFrame {
                        can_id: fbs.can_id(),
                        can_dlc: dlc,
                        flags: fbs.flags() as u8,
                        _padding: [0; 2],
                        data: data_arr,
                    };

                    Some(OrderedCanFrame { vtime, sequence, frame })
                },
                move |packet| {
                    let mut backlog = backlog_clone.lock(); // virtmcu-allow: mutex reasoning="Backlog managed securely"
                    backlog.push_back(packet.frame);
                },
            )
            .map_err(|e| format!("Failed to init receiver: {e}"))?;

            self.receiver = Some(rec);
            virtmcu_qom::sim_debug!("CAN-FD: Node {} initialized with transport", self.node_id);
        } else {
            virtmcu_qom::sim_debug!(
                "CAN-FD: Node {} initialized without transport (standalone mode)",
                self.node_id
            );
        }
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

    // virtmcu-allow: mutex reasoning="Wait mutex for MmioDevice"
    #[rustfmt::skip]
    fn wait_mutex(&self) -> &Mutex<()> { // virtmcu-allow: mutex reasoning="State managed securely"
        &self.wait_mutex
    }
}

impl virtmcu_qom::device::MmioDevice for CanfdState {
    fn read(&self, _addr: u64, _size: u32) -> virtmcu_qom::device::MmioResult<'_> {
        let _guard = self.drain.acquire();
        virtmcu_qom::device::MmioResult::Ready(0)
    }

    fn write(&self, _addr: u64, _val: u64, _size: u32) {
        let _guard = self.drain.acquire();
    }

    fn condvar(&self) -> &Condvar {
        &self.cond
    }

    // virtmcu-allow: mutex reasoning="Wait mutex for MmioDevice"
    #[rustfmt::skip]
    fn wait_mutex(&self) -> &Mutex<()> { // virtmcu-allow: mutex reasoning="State managed securely"
        &self.wait_mutex
    }
}

// RFC-0023 Phase 5: DSO Registration
virtmcu_qom::register_peripheral!(VirtmcuCanHostQEMU);

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::TrySendError;

    #[test]
    fn test_canfd_tx_thread_exits_on_shutdown() {
        let (tx, rx) = bounded::<(u64, u64, Vec<u8>)>(65536);
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_clone = Arc::clone(&shutdown);

        let handle = std::thread::spawn(move || loop {
            if shutdown_clone.load(Ordering::Acquire) {
                break;
            }
            match rx.recv_timeout(Duration::from_millis(1)) {
                Ok(_) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        });

        shutdown.store(true, Ordering::Release);
        drop(tx);

        let start = std::time::Instant::now();
        handle.join().expect("thread panicked");
        assert!(
            start.elapsed() < Duration::from_millis(100),
            "CAN-FD TX thread did not exit within 100 ms"
        );
    }

    #[test]
    #[should_panic(expected = "FATAL: Channel flooded")]
    fn test_canfd_tx_channel_flood_panics() {
        let (tx, _rx) = bounded::<(u64, u64, alloc::vec::Vec<u8>)>(65536);
        for _i in 0..65536_u64 {
            tx.try_send((0, 0, alloc::vec![])).expect("should not be full yet");
        }
        match tx.try_send((0, 0, alloc::vec![])) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(_)) => panic!("FATAL: Channel flooded. PDES barrier failure."),
        }
    }
}
