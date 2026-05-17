#![allow(clippy::all, unused_imports, dead_code, unused_variables, unused_mut)] // virtmcu-allow: allow reasoning="Zero unsafe"
#![allow(clippy::all)] // virtmcu-allow: allow reasoning="Zero unsafe"
#![allow(clippy::panic)] // virtmcu-allow: allow reasoning="Fail Loudly"
#![allow(clippy::not_unsafe_ptr_arg_deref)]
// virtmcu-allow: allow reasoning="Zero unsafe"
// virtmcu-allow: allow reasoning="Pending P1 migration: deref_qom_ptr/opaque_to_state replaced by dynamic_cast_qom"
#![allow(deprecated)]
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
// std is required: zenoh/tokio bring std
//! Virtmcu 802.15.4 radio with pluggable transport.

extern crate alloc;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use core::ffi::c_void;
use virtmcu_qom::irq::{qemu_set_irq, QemuIrq};
use virtmcu_qom::memory::MemoryRegion;
use virtmcu_qom::qdev::{sysbus_init_irq, SysBusDevice};
use virtmcu_qom::sync::{Condvar, DeliveryPacket, Mutex, VcpuDrain, VtimeIngress};
use virtmcu_qom::timer::{QomTimer, QEMU_CLOCK_VIRTUAL};
use virtmcu_wire::{DataTransport, Rf802154Mhr};

const IEEE_FIFO_SIZE: usize = 128;
const IEEE_DEFAULT_BE: u8 = 3;

// Register Offsets
const REG_TX_DATA: u64 = 0x00;
const REG_TX_LEN: u64 = 0x04;
const REG_TX_GO: u64 = 0x08;
const REG_RX_DATA: u64 = 0x0C;
const REG_RX_LEN: u64 = 0x10;
const REG_STATUS: u64 = 0x14;
const REG_RSSI: u64 = 0x18;
const REG_STATE: u64 = 0x1C;
const REG_PAN_ID: u64 = 0x20;
const REG_SHORT_ADDR: u64 = 0x24;
const REG_EXT_ADDR_LO: u64 = 0x28;
const REG_EXT_ADDR_HI: u64 = 0x2C;

// Status bits
const STATUS_RX_PENDING: u32 = 0x01;
const STATUS_TX_DONE: u32 = 0x02;

// Masks and Shifts

const STATE_SHIFT: u32 = 8;
const ADDR_32_MASK: u64 = 0xFFFFFFFF;
const ADDR_32_SHIFT: u32 = 32;

// Addressing
const IEEE_BROADCAST_PAN: u16 = 0xFFFF;
const IEEE_BROADCAST_ADDR: u16 = 0xFFFF;
const IEEE_ADDR_MODE_NONE: u16 = 0x00;
const IEEE_ADDR_MODE_SHORT: u16 = 0x02;
const IEEE_ADDR_MODE_EXT: u16 = 0x03;
const IEEE_ADDR_MODE_SHIFT: u32 = 10;
const IEEE_ADDR_MODE_MASK: u16 = 0x03;

// Frame and Timing
const IEEE_NS_PER_BYTE: u64 = 32_000;
const IEEE_OVERHEAD_BYTES: u64 = 6;
const IEEE_ACK_REQUEST_BIT: u16 = 1 << 5;
const IEEE_ACK_FRAME_TYPE: u8 = 0x02;
const IEEE_DEFAULT_LQI: u8 = 255;

// FNV-1a constants for deterministic random
const FNV_OFFSET_BASIS: u32 = 0x811c9dc5;
const FNV_PRIME: u32 = 0x01000193;

// Backoff and Timing
const MAC_MIN_BE: u8 = 3;
const UNIT_BACKOFF_PERIOD_NS: u64 = 320_000; // 20 symbols * 16 us/symbol
const SIFS_NS: u64 = 192_000; // 12 symbols * 16 us/symbol

// Byte slicing for deterministic_random
const NODE_ID_OFFSET: usize = 0;
const NODE_ID_SIZE: usize = 4;
const VTIME_OFFSET: usize = 4;
const VTIME_SIZE: usize = 8;
const EXTRA_OFFSET: usize = 12;
const EXTRA_SIZE: usize = 8;
const HASH_BYTES_LEN: usize = 20;

// ACK frame
const ACK_RESERVED_BYTE: u8 = 0x00;

#[repr(C)]
#[derive(virtmcu_qom::MmioDevice)]
// virtmcu-allow: magic_numbers reasoning="QOM device name"
#[virtmcu_qom::macros::qom_device(name = "virtmcu-ieee-mac")]
pub struct VirtmcuIeeeQEMU {
    pub parent_obj: SysBusDevice,
    pub iomem: MemoryRegion,
    pub irq: QemuIrq,

    /* Properties */
    #[qom_property]
    pub node_id: u32,
    #[qom_link(target = "virtmcu-transport-hub")]
    pub transport_hub: virtmcu_qom::qom::QomLink<dyn DataTransport>,
    #[qom_property]
    pub router: virtmcu_qom::qom::QomString,
    #[qom_property]
    pub topic: virtmcu_qom::qom::QomString,
    #[qom_property]
    pub debug: bool,

    /* Rust state */
    #[qom_state]
    pub state: VirtmcuIeeeState,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct RxFrame {
    delivery_vtime: u64,
    sequence: u64,
    data: [u8; 128],
    size: usize,
    rssi: i8,
}

impl DeliveryPacket for RxFrame {
    fn delivery_vtime_ns(&self) -> u64 {
        self.delivery_vtime
    }
}

#[repr(u8)]
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum RadioState {
    Off = 0,
    Idle = 1,
    Rx = 2,
    Tx = 3,
}

pub struct VirtmcuIeeeState {
    parent_ptr: *mut VirtmcuIeeeQEMU,
    cond: Arc<Condvar>,
    wait_mutex: Arc<Mutex<()>>, // virtmcu-allow: mutex reasoning="State managed securely"
    pub drain: VcpuDrain,
    transport: Option<Arc<dyn virtmcu_wire::DataTransport>>,
    topic_tx: String,
    topic_rx: String,
    receiver: Option<VtimeIngress<RxFrame>>,
    backoff_timer: Option<QomTimer>,
    ack_timer: Option<QomTimer>,
    tx_timer: Option<QomTimer>,
    generation: Arc<core::sync::atomic::AtomicU64>,

    // All state accessed securely; see Mutex docs.
    inner: Arc<Mutex<Virtmcu802154Inner>>, // virtmcu-allow: mutex reasoning="State managed securely"
    pub _liveliness: Option<Box<dyn virtmcu_wire::LivelinessToken>>,
}

struct Virtmcu802154Inner {
    node_id: u32,
    tx_fifo: [u8; 128],
    tx_len: u32,
    rx_fifo: [u8; 128],
    rx_len: u32,
    rx_read_pos: u32,
    rx_rssi: i8,
    status: u32,
    state: RadioState,

    pan_id: u16,
    short_addr: u16,
    ext_addr: u64,

    // CSMA/CA state
    nb: u8,
    be: u8,

    // Auto-ACK state
    ack_pending: bool,
    ack_seq: u8,
    tx_sequence: u64,
}

impl virtmcu_qom::device::PeripheralState for VirtmcuIeeeState {
    type QomType = VirtmcuIeeeQEMU;

    fn new(qemu_dev: &Self::QomType) -> Self {
        virtmcu_qom::ffi_call! {
            let dev_mut = core::ptr::from_ref(qemu_dev).cast_mut().cast::<SysBusDevice>();
            let irq_ptr = &raw mut (*core::ptr::from_ref(qemu_dev).cast_mut()).irq;
            sysbus_init_irq(dev_mut, irq_ptr);
        }

        let parent_ptr = core::ptr::from_ref(qemu_dev).cast_mut();
        let transport = qemu_dev.transport_hub.get();

        let topic_tx;
        let topic_rx;
        let topic_str =
            if qemu_dev.topic.is_null() { None } else { Some(qemu_dev.topic.as_string()) };

        if let Some(t) = topic_str {
            topic_tx = alloc::format!("{t}/tx");
            topic_rx = alloc::format!("{t}/rx");
        } else {
            topic_tx = alloc::format!("sim/rf/ieee802154/{}/tx", qemu_dev.node_id);
            topic_rx = alloc::format!("sim/rf/ieee802154/{}/rx", qemu_dev.node_id);
        }

        Self {
            parent_ptr,
            cond: Arc::new(Condvar::new()),
            wait_mutex: Arc::new(Mutex::new(())), // virtmcu-allow: mutex reasoning="State managed securely"
            drain: VcpuDrain::new(),
            transport,
            topic_tx,
            topic_rx,
            receiver: None,
            backoff_timer: None,
            ack_timer: None,
            tx_timer: None,
            generation: Arc::new(core::sync::atomic::AtomicU64::new(0)),
            inner: Arc::new(Mutex::new(Virtmcu802154Inner {
                // virtmcu-allow: mutex reasoning="State managed securely"
                node_id: qemu_dev.node_id,
                tx_fifo: [0; IEEE_FIFO_SIZE],
                tx_len: 0,
                rx_fifo: [0; IEEE_FIFO_SIZE],
                rx_len: 0,
                rx_read_pos: 0,
                rx_rssi: 0,
                status: 0,
                state: RadioState::Idle,
                pan_id: IEEE_BROADCAST_PAN,
                short_addr: IEEE_BROADCAST_ADDR,
                ext_addr: 0,
                nb: 0,
                be: IEEE_DEFAULT_BE,
                ack_pending: false,
                ack_seq: 0,
                tx_sequence: 0,
            })),
            _liveliness: None,
        }
    }
}

fn decode_ieee802154(opaque: *mut c_void, _topic: &str, data: &[u8]) -> Option<RxFrame> {
    let state = virtmcu_qom::ffi_call! { &*(opaque as *mut VirtmcuIeeeState) };
    let inner = state.inner.lock();

    let (vtime, sequence, data) = virtmcu_wire::decode_frame(data)?;
    let frame = virtmcu_wire::rf802154::size_prefixed_root_as_rf_802154_frame(data).ok()?;

    let rssi = frame.rssi();

    let mhr = Rf802154Mhr {
        fcf: frame.fcf(),
        seq_num: frame.mhr_seq_num(),
        dest_pan: frame.dest_pan(),
        dest_addr: frame.dest_addr(),
        src_pan: frame.src_pan(),
        src_addr: frame.src_addr(),
    };

    let frame_data = frame.data()?.bytes();

    let size = frame_data.len();
    if size > IEEE_FIFO_SIZE {
        return None;
    }

    if !frame_matches_address(inner.pan_id, inner.short_addr, inner.ext_addr, &mhr) {
        return None;
    }

    let mut stored_data = [0u8; IEEE_FIFO_SIZE];
    stored_data[..size].copy_from_slice(frame_data);

    Some(RxFrame { delivery_vtime: vtime, sequence, data: stored_data, size, rssi })
}

fn deliver_ieee802154(opaque: *mut c_void, frame: RxFrame) {
    let state = virtmcu_qom::timer::opaque_to_state::<VirtmcuIeeeState>(opaque);
    let mut inner = state.inner.lock();

    // Re-parse MHR for ACK handling
    let mhr = Rf802154Mhr::parse(&frame.data[..frame.size]);
    if (mhr.fcf & IEEE_ACK_REQUEST_BIT) != 0 {
        inner.ack_pending = true;
        inner.ack_seq = mhr.seq_num;
        if let Some(ack_timer) = &state.ack_timer {
            ack_timer.mod_ns((frame.delivery_vtime + SIFS_NS) as i64);
        }
    }

    if inner.state == RadioState::Rx && (inner.status & STATUS_RX_PENDING == 0) {
        inner.rx_fifo[..frame.size].copy_from_slice(&frame.data[..frame.size]);
        inner.rx_len = frame.size as u32;
        inner.rx_rssi = frame.rssi;
        inner.rx_read_pos = 0;
        inner.status |= STATUS_RX_PENDING;

        virtmcu_qom::sim_info!("deliver_ieee802154: frame delivered to FIFO, len={}", frame.size);

        virtmcu_qom::ffi_call! {
            qemu_set_irq((*state.parent_ptr).irq, 1);
        }
        let _guard = state.wait_mutex.lock();
        state.cond.notify_all();
    } else if inner.state == RadioState::Rx {
        virtmcu_qom::sim_info!("deliver_ieee802154: frame dropped (RX_PENDING)");
    }
}

impl virtmcu_qom::device::Peripheral for VirtmcuIeeeState {
    fn realize(&mut self, _ctx: &virtmcu_qom::device::BqlContext) -> Result<(), String> {
        let state_ptr = core::ptr::from_mut::<VirtmcuIeeeState>(self);

        if let Some(transport) = &self.transport {
            match VtimeIngress::new(
                &**transport,
                &self.topic_rx,
                Arc::clone(&self.generation),
                state_ptr as *mut c_void,
                decode_ieee802154,
                deliver_ieee802154,
            ) {
                Ok(receiver) => self.receiver = Some(receiver),
                Err(e) => {
                    return Err(format!(
                        "ieee802154: failed to initialize VtimeIngress for topic {}: {}",
                        self.topic_rx, e
                    ));
                }
            }

            let node_id = virtmcu_qom::timer::deref_qom_ptr_const::<VirtmcuIeeeQEMU>(
                self.parent_ptr as *mut core::ffi::c_void,
            )
            .node_id;
            let hb_topic = format!("sim/ieee802154/liveliness/{node_id}");
            self._liveliness = transport.declare_liveliness(&hb_topic);

            virtmcu_qom::sim_info!(
                "ieee802154 initialized for node {} on topic {}",
                node_id,
                self.topic_rx
            );
        }

        self.backoff_timer = Some(virtmcu_qom::ffi_call! {
            QomTimer::new_safe(QEMU_CLOCK_VIRTUAL, backoff_timer_cb, state_ptr as *mut c_void)
        });

        self.ack_timer = Some(virtmcu_qom::ffi_call! {
            QomTimer::new_safe(QEMU_CLOCK_VIRTUAL, ack_timer_cb, state_ptr as *mut c_void)
        });
        self.tx_timer = Some(virtmcu_qom::ffi_call! {
            QomTimer::new_safe(QEMU_CLOCK_VIRTUAL, tx_timer_cb, state_ptr as *mut c_void)
        });

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

    fn reset(&mut self) {
        let mut inner = self.inner.lock();

        inner.tx_len = 0;
        inner.rx_len = 0;
        inner.rx_read_pos = 0;
        inner.rx_rssi = 0;
        inner.status = 0;
        inner.state = RadioState::Idle;
        inner.nb = 0;
        inner.be = MAC_MIN_BE;
        inner.ack_pending = false;

        if let Some(timer) = &self.backoff_timer {
            timer.del();
        }
        if let Some(timer) = &self.ack_timer {
            timer.del();
        }
        if let Some(timer) = &self.tx_timer {
            timer.del();
        }
    }

    fn condvar(&self) -> &Condvar {
        &self.cond
    }

    #[rustfmt::skip]
    fn wait_mutex(&self) -> &Mutex<()> { // virtmcu-allow: mutex reasoning="State managed securely"
        &self.wait_mutex
    }
}

impl virtmcu_qom::device::MmioDevice for VirtmcuIeeeState {
    fn read(&self, offset: u64, _size: u32) -> virtmcu_qom::device::MmioResult<'_> {
        let _guard = self.drain.acquire();
        let mut inner = self.inner.lock();
        match offset {
            REG_TX_LEN => virtmcu_qom::device::MmioResult::Ready(u64::from(inner.tx_len)),
            REG_RX_DATA
                if (inner.status & STATUS_RX_PENDING != 0)
                    && (inner.rx_read_pos < inner.rx_len) =>
            {
                let val = u64::from(inner.rx_fifo[inner.rx_read_pos as usize]);
                inner.rx_read_pos += 1;
                virtmcu_qom::device::MmioResult::Ready(val)
            }
            REG_RX_LEN => virtmcu_qom::device::MmioResult::Ready(u64::from(inner.rx_len)),
            REG_STATUS => {
                let status = inner.status | ((inner.state as u32) << STATE_SHIFT);
                let waiting_rx = status & STATUS_RX_PENDING == 0 && inner.state == RadioState::Rx;
                let waiting_tx = status & STATUS_TX_DONE == 0 && inner.state == RadioState::Tx;
                if waiting_rx || waiting_tx {
                    drop(inner);
                    let inner_clone1 = Arc::clone(&self.inner);
                    let inner_clone2 = Arc::clone(&self.inner);
                    let inner_clone3 = Arc::clone(&self.inner);
                    return virtmcu_qom::device::MmioResult::wait_for(
                        move || {
                            let in2 = inner_clone1.lock();
                            let s2 = in2.status | ((in2.state as u32) << STATE_SHIFT);
                            let w_rx = s2 & STATUS_RX_PENDING == 0 && in2.state == RadioState::Rx;
                            let w_tx = s2 & STATUS_TX_DONE == 0 && in2.state == RadioState::Tx;
                            !(w_rx || w_tx)
                        },
                        move || {
                            let in3 = inner_clone2.lock();
                            u64::from(in3.status | ((in3.state as u32) << STATE_SHIFT))
                        },
                        move || {
                            let in4 = inner_clone3.lock();
                            u64::from(in4.status | ((in4.state as u32) << STATE_SHIFT))
                        },
                    );
                }
                virtmcu_qom::device::MmioResult::Ready(u64::from(status))
            }
            REG_RSSI => virtmcu_qom::device::MmioResult::Ready(u64::from(inner.rx_rssi as u8)),
            REG_STATE => virtmcu_qom::device::MmioResult::Ready(inner.state as u64),
            REG_PAN_ID => virtmcu_qom::device::MmioResult::Ready(u64::from(inner.pan_id)),
            REG_SHORT_ADDR => virtmcu_qom::device::MmioResult::Ready(u64::from(inner.short_addr)),
            REG_EXT_ADDR_LO => {
                virtmcu_qom::device::MmioResult::Ready(inner.ext_addr & ADDR_32_MASK)
            }
            REG_EXT_ADDR_HI => {
                virtmcu_qom::device::MmioResult::Ready(inner.ext_addr >> ADDR_32_SHIFT)
            }
            _ => {
                let parent = virtmcu_qom::timer::deref_qom_ptr_const::<VirtmcuIeeeQEMU>(
                    self.parent_ptr as *mut core::ffi::c_void,
                );
                if parent.debug {
                    virtmcu_qom::sim_debug!("ieee802154_read: unhandled offset 0x{:x}", offset);
                }
                virtmcu_qom::device::MmioResult::Ready(0)
            }
        }
    }

    fn write(&self, offset: u64, value: u64, _size: u32) {
        let _guard = self.drain.acquire();
        let mut inner = self.inner.lock();
        match offset {
            REG_TX_DATA => {
                if inner.tx_len < IEEE_FIFO_SIZE as u32 {
                    let len = inner.tx_len as usize;
                    inner.tx_fifo[len] = value as u8;
                    inner.tx_len += 1;
                }
            }
            REG_TX_GO => {
                inner.state = RadioState::Tx;
                inner.status &= !STATUS_TX_DONE;
                inner.nb = 0;
                inner.be = MAC_MIN_BE;
                schedule_backoff(self.backoff_timer.as_ref(), &mut inner);
            }
            REG_STATE => {
                const STATE_RX: u64 = 2;
                const STATE_TX: u64 = 3;
                inner.state = match value {
                    0 => RadioState::Off,
                    1 => RadioState::Idle,
                    STATE_RX => RadioState::Rx, // virtmcu-allow: magic_numbers reasoning="State enum mapping"
                    STATE_TX => RadioState::Tx, // virtmcu-allow: magic_numbers reasoning="State enum mapping"
                    _ => inner.state,
                };
            }
            REG_STATUS => {
                if value as u32 & STATUS_TX_DONE != 0 {
                    inner.status &= !STATUS_TX_DONE;
                }
                if value as u32 & STATUS_RX_PENDING != 0 {
                    inner.status &= !STATUS_RX_PENDING;
                    inner.rx_len = 0;
                    inner.rx_read_pos = 0;
                    virtmcu_qom::ffi_call! {
                        qemu_set_irq((*self.parent_ptr).irq, 0);
                    }
                }
            }
            REG_PAN_ID => inner.pan_id = value as u16,
            REG_SHORT_ADDR => inner.short_addr = value as u16,
            REG_EXT_ADDR_LO => {
                inner.ext_addr = (inner.ext_addr & !(ADDR_32_MASK)) | (value & ADDR_32_MASK);
            }
            REG_EXT_ADDR_HI => {
                inner.ext_addr = (inner.ext_addr & ADDR_32_MASK) | (value << ADDR_32_SHIFT);
            }
            _ => {
                let parent = virtmcu_qom::timer::deref_qom_ptr_const::<VirtmcuIeeeQEMU>(
                    self.parent_ptr as *mut core::ffi::c_void,
                );
                if parent.debug {
                    virtmcu_qom::sim_debug!(
                        "ieee802154_write: unhandled offset 0x{:x} val 0x{:x}",
                        offset,
                        value
                    );
                }
            }
        }
    }

    fn condvar(&self) -> &Condvar {
        &self.cond
    }

    #[rustfmt::skip]
    fn wait_mutex(&self) -> &Mutex<()> { // virtmcu-allow: mutex reasoning="State managed securely"
        &self.wait_mutex
    }
}

const MAC_MAX_BE: u8 = 5;
const MAC_MAX_CSMA_BACKOFFS: u8 = 4;

fn deterministic_random(node_id: u32, vtime_ns: u64, extra: u64) -> u32 {
    let mut hash = FNV_OFFSET_BASIS;
    let mut bytes = [0u8; HASH_BYTES_LEN];
    bytes[NODE_ID_OFFSET..NODE_ID_OFFSET + NODE_ID_SIZE].copy_from_slice(&node_id.to_le_bytes());
    bytes[VTIME_OFFSET..VTIME_OFFSET + VTIME_SIZE].copy_from_slice(&vtime_ns.to_le_bytes());
    bytes[EXTRA_OFFSET..EXTRA_OFFSET + EXTRA_SIZE].copy_from_slice(&extra.to_le_bytes());
    for byte in bytes {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

fn schedule_backoff(backoff_timer: Option<&QomTimer>, inner: &mut Virtmcu802154Inner) {
    let max_backoff = (1u32 << inner.be) - 1;
    let now =
        virtmcu_qom::timer::qemu_clock_get_ns_safe(virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL, unsafe {
            &virtmcu_qom::device::BqlContext::new_unchecked()
        }) as u64;
    let rand_val = deterministic_random(inner.node_id, now, inner.tx_sequence);
    let backoff_count = rand_val % (max_backoff + 1);
    let delay_ns = u64::from(backoff_count) * UNIT_BACKOFF_PERIOD_NS;

    if let Some(timer) = backoff_timer {
        timer.mod_ns((now + delay_ns) as i64);
    }
}

fn tx_real(
    transport: &dyn virtmcu_wire::DataTransport,
    topic: &str,
    tx_timer: Option<&QomTimer>,
    inner: &mut Virtmcu802154Inner,
) {
    let vtime =
        virtmcu_qom::timer::qemu_clock_get_ns_safe(virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL, unsafe {
            &virtmcu_qom::device::BqlContext::new_unchecked()
        }) as u64;
    let seq = inner.tx_sequence;
    inner.tx_sequence += 1;
    let payload = &inner.tx_fifo[..inner.tx_len as usize];
    let mhr = Rf802154Mhr::parse(payload);

    let mut builder = flatbuffers::FlatBufferBuilder::with_capacity(128);
    let data = builder.create_vector(payload);

    let args = virtmcu_wire::rf802154::Rf802154FrameArgs {
        delivery_vtime_ns: vtime,
        sequence_number: seq,
        rssi: 0,
        lqi: IEEE_DEFAULT_LQI,
        fcf: mhr.fcf,
        mhr_seq_num: mhr.seq_num,
        dest_pan: mhr.dest_pan,
        dest_addr: mhr.dest_addr,
        src_pan: mhr.src_pan,
        src_addr: mhr.src_addr,
        data: Some(data),
    };

    let frame = virtmcu_wire::rf802154::Rf802154Frame::create(&mut builder, &args);
    builder.finish_size_prefixed(frame, None);
    let msg = builder.finished_data();

    match transport.reserve(topic, msg.len()) {
        Ok(mut reservation) => {
            reservation.buffer_mut().copy_from_slice(msg);
            let _ = reservation.commit(vtime, seq);
        }
        Err(e) => virtmcu_qom::sim_err!("IEEE802154: failed to reserve tx_real: {:?}", e),
    }

    let air_time_ns = (IEEE_OVERHEAD_BYTES + inner.tx_len as u64) * IEEE_NS_PER_BYTE;

    if let Some(timer) = tx_timer {
        timer.mod_ns((vtime + air_time_ns) as i64);
    }
}

// virtmcu-allow: extern_c_timer_cb reasoning="Pending ClosureTimer migration in P1"
extern "C" fn tx_timer_cb(opaque: *mut c_void) {
    let s = virtmcu_qom::timer::opaque_to_state::<VirtmcuIeeeState>(opaque);
    let mut inner = s.inner.lock();

    inner.tx_len = 0;
    inner.status |= STATUS_TX_DONE;
    inner.state = RadioState::Idle;
    virtmcu_qom::ffi_call! {
        qemu_set_irq((*s.parent_ptr).irq, 1);
    }
}

// virtmcu-allow: extern_c_timer_cb reasoning="Pending ClosureTimer migration in P1"
extern "C" fn backoff_timer_cb(opaque: *mut c_void) {
    let s = virtmcu_qom::timer::opaque_to_state::<VirtmcuIeeeState>(opaque);
    let mut inner = s.inner.lock();
    let _now =
        virtmcu_qom::timer::qemu_clock_get_ns_safe(virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL, unsafe {
            &virtmcu_qom::device::BqlContext::new_unchecked()
        }) as u64;

    let busy = (inner.status & STATUS_RX_PENDING) != 0;

    if busy {
        inner.nb += 1;
        if inner.nb > MAC_MAX_CSMA_BACKOFFS {
            inner.tx_len = 0;
            inner.state = RadioState::Idle;
            inner.status |= STATUS_TX_DONE;
            virtmcu_qom::ffi_call! {
                qemu_set_irq((*s.parent_ptr).irq, 1);
            }
        } else {
            inner.be = core::cmp::min(inner.be + 1, MAC_MAX_BE);
            schedule_backoff(s.backoff_timer.as_ref(), &mut inner);
        }
    } else if let Some(transport) = &s.transport {
        tx_real(&**transport, &s.topic_tx, s.tx_timer.as_ref(), &mut inner);
    }
}

// virtmcu-allow: extern_c_timer_cb reasoning="Pending ClosureTimer migration in P1"
extern "C" fn ack_timer_cb(opaque: *mut c_void) {
    let s = virtmcu_qom::timer::opaque_to_state::<VirtmcuIeeeState>(opaque);
    let mut inner = s.inner.lock();

    if !inner.ack_pending {
        return;
    }

    let now =
        virtmcu_qom::timer::qemu_clock_get_ns_safe(virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL, unsafe {
            &virtmcu_qom::device::BqlContext::new_unchecked()
        }) as u64;
    let seq = inner.tx_sequence;
    inner.tx_sequence += 1;
    let ack_payload = [IEEE_ACK_FRAME_TYPE, ACK_RESERVED_BYTE, inner.ack_seq];
    let mhr = Rf802154Mhr::parse(&ack_payload);

    let mut builder = flatbuffers::FlatBufferBuilder::with_capacity(128);
    let data = builder.create_vector(&ack_payload);

    let args = virtmcu_wire::rf802154::Rf802154FrameArgs {
        delivery_vtime_ns: now,
        sequence_number: seq,
        rssi: 0,
        lqi: IEEE_DEFAULT_LQI,
        fcf: mhr.fcf,
        mhr_seq_num: mhr.seq_num,
        dest_pan: mhr.dest_pan,
        dest_addr: mhr.dest_addr,
        src_pan: mhr.src_pan,
        src_addr: mhr.src_addr,
        data: Some(data),
    };

    let frame = virtmcu_wire::rf802154::Rf802154Frame::create(&mut builder, &args);
    builder.finish_size_prefixed(frame, None);
    let msg = builder.finished_data();

    if let Some(transport) = &s.transport {
        match transport.reserve(&s.topic_tx, msg.len()) {
            Ok(mut reservation) => {
                reservation.buffer_mut().copy_from_slice(msg);
                let _ = reservation.commit(now, seq);
            }
            Err(e) => virtmcu_qom::sim_err!("IEEE802154: failed to reserve ack_timer_cb: {:?}", e),
        }
    }
    inner.ack_pending = false;
}

fn frame_matches_address(pan_id: u16, short_addr: u16, ext_addr: u64, mhr: &Rf802154Mhr) -> bool {
    let dest_addr_mode = (mhr.fcf >> IEEE_ADDR_MODE_SHIFT) & IEEE_ADDR_MODE_MASK;

    match dest_addr_mode {
        IEEE_ADDR_MODE_NONE => true,
        IEEE_ADDR_MODE_SHORT => {
            let pan_matches = mhr.dest_pan == IEEE_BROADCAST_PAN || mhr.dest_pan == pan_id;
            let addr_matches =
                mhr.dest_addr as u16 == IEEE_BROADCAST_ADDR || mhr.dest_addr as u16 == short_addr;
            pan_matches && addr_matches
        }
        IEEE_ADDR_MODE_EXT => {
            let pan_matches = mhr.dest_pan == IEEE_BROADCAST_PAN || mhr.dest_pan == pan_id;
            let addr_matches = mhr.dest_addr == ext_addr;
            pan_matches && addr_matches
        }
        _ => false,
    }
}

virtmcu_qom::register_peripheral!(VirtmcuIeeeQEMU);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_802154_qemu_layout() {
        assert_eq!(
            core::mem::offset_of!(VirtmcuIeeeQEMU, parent_obj),
            0,
            "SysBusDevice must be the first field"
        );
    }
}
