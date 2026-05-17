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
//! S32K144 LPUART peripheral for VirtMCU simulation with pluggable transport.

extern crate alloc;

use alloc::collections::VecDeque;
use alloc::sync::Arc;
use core::sync::atomic::AtomicU64;
use virtmcu_qom::irq::{qemu_set_irq, QemuIrq};
use virtmcu_qom::memory::MemoryRegion;
use virtmcu_qom::qdev::SysBusDevice;
use virtmcu_qom::sync::{Condvar, Mutex, VcpuDrain, VtimeIngress};
use virtmcu_qom::timer::{QomTimer, QEMU_CLOCK_VIRTUAL};
use virtmcu_wire::lin_generated::virtmcu::lin::{LinFrame, LinFrameArgs, LinMessageType};

const MAX_RX_FIFO: usize = 4;

/// S32K144 LPUART QEMU object structure
#[repr(C)]
#[derive(virtmcu_qom::MmioDevice)]
#[virtmcu_qom::macros::qom_device(name = "s32k144-lpuart")]
pub struct S32K144LpuartQemu {
    pub parent_obj: SysBusDevice,
    pub iomem: MemoryRegion,
    pub irq: QemuIrq,

    /* Properties */
    #[qom_property]
    pub node_id: u32,
    #[qom_property]
    pub transport: virtmcu_qom::qom::QomString,
    #[qom_property]
    pub router: virtmcu_qom::qom::QomString,
    #[qom_property]
    pub topic: virtmcu_qom::qom::QomString,
    #[qom_property]
    pub debug: bool,

    /* Links */
    #[qom_link(target = "virtmcu-transport-hub")]
    pub transport_hub: virtmcu_qom::qom::QomLink<dyn virtmcu_wire::DataTransport>,

    /* Rust state */
    #[qom_state]
    pub state: LpuartState,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct OrderedLinFrame {
    pub vtime: u64,
    pub msg_type: LinMessageType,
    pub data: alloc::vec::Vec<u8>,
}

impl virtmcu_qom::sync::DeliveryPacket for OrderedLinFrame {
    fn delivery_vtime_ns(&self) -> u64 {
        self.vtime
    }
}

pub struct LpuartState {
    pub node_id: u32,
    pub irq: QemuIrq,
    pub transport_ptr: Option<Arc<dyn virtmcu_wire::DataTransport>>,
    pub receiver: Option<VtimeIngress<OrderedLinFrame>>,

    pub cond: Arc<Condvar>,
    pub wait_mutex: Arc<Mutex<()>>, // virtmcu-allow: mutex reasoning="State managed securely"
    pub drain: VcpuDrain,

    pub inner: Arc<Mutex<LpuartInner>>, // virtmcu-allow: mutex reasoning="State managed securely"
    pub tx_timer: Option<QomTimer>,

    pub topic: alloc::string::String,
    pub tx_topic: alloc::string::String,
    pub rx_topic: alloc::string::String,
}

pub struct LpuartInner {
    // Registers
    pub baud: u32,
    pub stat: u32,
    pub ctrl: u32,
    pub _data: u32,
    pub match_: u32,
    pub modir: u32,
    pub fifo: u32,
    pub water: u32,

    // Internal state
    pub rx_buffer: alloc::vec::Vec<u8>,
    pub tx_fifo: VecDeque<u8>,
}

const REG_VERID: u64 = 0x00;
const REG_PARAM: u64 = 0x04;
const REG_GLOBAL: u64 = 0x08;
const REG_PINCFG: u64 = 0x0C;
const REG_BAUD: u64 = 0x10;
const REG_STAT: u64 = 0x14;
const REG_CTRL: u64 = 0x18;
const REG_DATA: u64 = 0x1C;
const REG_MATCH: u64 = 0x20;
const REG_MODIR: u64 = 0x24;
const REG_FIFO: u64 = 0x28;
const REG_WATER: u64 = 0x2C;

const LPUART_RESET_BAUD: u32 = 0x0F000004;
const LPUART_RESET_STAT: u32 = 0x00C00000;
const LPUART_RESET_FIFO: u32 = 0x00C00011;

const LPUART_DATA_MASK: u32 = 0xFF;

const STAT_LBKDIF: u32 = 1 << 31;
const STAT_TDRE: u32 = 1 << 23;
const STAT_TC: u32 = 1 << 22;
const STAT_RDRF: u32 = 1 << 21;
const STAT_IDLE: u32 = 1 << 20;
const STAT_OR: u32 = 1 << 19;
const STAT_NF: u32 = 1 << 18;
const STAT_FE: u32 = 1 << 17;
const STAT_PF: u32 = 1 << 16;

const CTRL_TIE: u32 = 1 << 23;
const CTRL_TCIE: u32 = 1 << 22;
const CTRL_RIE: u32 = 1 << 21;
const CTRL_ILIE: u32 = 1 << 20;
const CTRL_TE: u32 = 1 << 19;
const CTRL_RE: u32 = 1 << 18;
const CTRL_SBK: u32 = 1 << 0;

const BAUD_LBKDIE: u32 = 1 << 31;
const BAUD_LBKDE: u32 = 1 << 24;

const LPUART_VERID: u64 = 0x04010001;
const LPUART_PARAM: u64 = 0x00020202;
const LPUART_TX_FIFO_CAP: usize = 4096;
const LPUART_SBR_MASK: u32 = 0x1FFF;
const LPUART_OSR_MASK: u32 = 0x1F;
const LPUART_OSR_SHIFT: u32 = 24;
const LPUART_DEFAULT_CLOCK_HZ: u32 = 48_000_000;
const LPUART_DEFAULT_BAUD_DELAY_NS: i64 = 86800;
const LPUART_BITS_PER_CHAR: i64 = 10;
const LPUART_NS_PER_SEC: i64 = 1_000_000_000;

impl virtmcu_qom::device::PeripheralState for LpuartState {
    type QomType = S32K144LpuartQemu;

    fn new(qemu_dev: &Self::QomType) -> Self {
        virtmcu_qom::ffi_call! {
            let dev_mut = core::ptr::from_ref(qemu_dev).cast_mut().cast::<SysBusDevice>();
            let irq_ptr = &raw mut (*core::ptr::from_ref(qemu_dev).cast_mut()).irq;
            virtmcu_qom::qdev::sysbus_init_irq(dev_mut, irq_ptr);
        }

        let topic_str = if qemu_dev.topic.is_null() {
            "sim/lin".to_owned()
        } else {
            qemu_dev.topic.as_string()
        };

        let node_id = qemu_dev.node_id;
        let tx_topic = alloc::format!("{topic_str}/{node_id}/tx");
        let rx_topic = alloc::format!("{topic_str}/{node_id}/rx");

        Self {
            node_id,
            irq: qemu_dev.irq,
            transport_ptr: qemu_dev.transport_hub.get(),
            receiver: None,
            cond: Arc::new(Condvar::new()),
            wait_mutex: Arc::new(Mutex::new(())), // virtmcu-allow: mutex reasoning="State managed securely"
            drain: VcpuDrain::new(),
            inner: Arc::new(Mutex::new(LpuartInner {
                // virtmcu-allow: mutex reasoning="State managed securely"
                baud: LPUART_RESET_BAUD,
                stat: LPUART_RESET_STAT,
                ctrl: 0,
                _data: 0,
                match_: 0,
                modir: 0,
                fifo: LPUART_RESET_FIFO,
                water: 0,
                rx_buffer: alloc::vec::Vec::new(), // virtmcu-allow: mutex reasoning="State managed securely"
                tx_fifo: VecDeque::new(),
            })),
            tx_timer: None,
            topic: topic_str,
            tx_topic,
            rx_topic,
        }
    }
}

impl virtmcu_qom::device::Peripheral for LpuartState {
    fn realize(
        &mut self,
        _ctx: &virtmcu_qom::device::BqlContext,
    ) -> Result<(), alloc::string::String> {
        let state_ptr = core::ptr::from_mut(self).cast::<core::ffi::c_void>();
        self.tx_timer = Some(virtmcu_qom::ffi_call! {
            QomTimer::new_safe(QEMU_CLOCK_VIRTUAL, lpuart_tx_timer_cb, state_ptr)
        });

        if let Some(t) = &self.transport_ptr {
            let inner_clone = Arc::clone(&self.inner);
            let irq_ptr = self.irq as usize;

            let rec = VtimeIngress::new_safe(
                &**t,
                &self.rx_topic,
                Arc::new(AtomicU64::new(0)),
                |_topic, payload| {
                    if let Some((vtime, _, data)) = virtmcu_wire::decode_frame(payload) {
                        if let Ok(frame) =
                            virtmcu_wire::lin_generated::virtmcu::lin::root_as_lin_frame(data)
                        {
                            let msg_type = frame.type_();
                            let data = frame.data().map(|d| d.iter().collect()).unwrap_or_default();
                            Some(OrderedLinFrame { vtime, msg_type, data })
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                },
                move |packet| {
                    let mut inner = inner_clone.lock();
                    match packet.msg_type {
                        LinMessageType::Sync => {
                            inner.rx_buffer.clear();
                            inner.rx_buffer.extend_from_slice(&packet.data);
                            inner.stat |= STAT_RDRF;
                        }
                        LinMessageType::Break if inner.baud & BAUD_LBKDE != 0 => {
                            inner.stat |= STAT_LBKDIF;
                        }
                        LinMessageType::Data if inner.ctrl & CTRL_RE != 0 => {
                            for byte in packet.data {
                                if inner.rx_buffer.len() >= MAX_RX_FIFO {
                                    inner.stat |= STAT_OR;
                                } else {
                                    inner.rx_buffer.push(byte);
                                }
                            }
                            if !inner.rx_buffer.is_empty() {
                                inner.stat |= STAT_RDRF;
                            }
                        }
                        _ => {}
                    }
                    let irq = irq_ptr as QemuIrq;
                    update_irqs(irq, &inner);
                },
            )
            .map_err(|e| alloc::format!("Failed to init receiver: {e}"))?;

            self.receiver = Some(rec);
            virtmcu_qom::sim_info!("LPUART Node {} subscribed to {}", self.node_id, self.rx_topic);
        }
        Ok(())
    }

    fn reset(&mut self) {
        let mut inner = self.inner.lock();
        inner.baud = LPUART_RESET_BAUD;
        inner.stat = LPUART_RESET_STAT;
        inner.ctrl = 0;
        inner.match_ = 0;
        inner.modir = 0;
        inner.fifo = LPUART_RESET_FIFO;
        inner.water = 0;

        inner.rx_buffer.clear();
        inner.tx_fifo.clear();

        if let Some(timer) = &self.tx_timer {
            timer.del();
        }
    }

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

    fn condvar(&self) -> &Condvar {
        &self.cond
    }

    #[rustfmt::skip]
    fn wait_mutex(&self) -> &Mutex<()> { // virtmcu-allow: mutex reasoning="State managed securely"
        &self.wait_mutex
    }
}

impl virtmcu_qom::device::MmioDevice for LpuartState {
    fn read(&self, addr: u64, _size: u32) -> virtmcu_qom::device::MmioResult<'_> {
        let _guard = self.drain.acquire();
        let mut inner = self.inner.lock();

        match addr {
            REG_VERID => virtmcu_qom::device::MmioResult::Ready(LPUART_VERID),
            REG_PARAM => virtmcu_qom::device::MmioResult::Ready(LPUART_PARAM),
            REG_BAUD => virtmcu_qom::device::MmioResult::Ready(u64::from(inner.baud)),
            REG_STAT => virtmcu_qom::device::MmioResult::Ready(u64::from(inner.stat)),
            REG_CTRL => virtmcu_qom::device::MmioResult::Ready(u64::from(inner.ctrl)),
            REG_DATA => {
                let val = if inner.rx_buffer.is_empty() {
                    0
                } else {
                    let byte = inner.rx_buffer.remove(0);
                    if inner.rx_buffer.is_empty() {
                        inner.stat &= !STAT_RDRF;
                    }
                    u32::from(byte)
                };
                virtmcu_qom::device::MmioResult::Ready(u64::from(val))
            }
            REG_MATCH => virtmcu_qom::device::MmioResult::Ready(u64::from(inner.match_)),
            REG_MODIR => virtmcu_qom::device::MmioResult::Ready(u64::from(inner.modir)),
            REG_FIFO => virtmcu_qom::device::MmioResult::Ready(u64::from(inner.fifo)),
            REG_WATER => virtmcu_qom::device::MmioResult::Ready(u64::from(inner.water)),
            _ => virtmcu_qom::device::MmioResult::Ready(0),
        }
    }

    fn write(&self, addr: u64, val: u64, _size: u32) {
        let _guard = self.drain.acquire();
        let mut inner = self.inner.lock();
        let val32 = val as u32;

        match addr {
            REG_BAUD => inner.baud = val32,
            REG_STAT => {
                inner.stat &=
                    !(val32 & (STAT_LBKDIF | STAT_OR | STAT_NF | STAT_FE | STAT_PF | STAT_IDLE));
            }
            REG_CTRL => {
                let old_ctrl = inner.ctrl;
                inner.ctrl = val32;
                if (inner.ctrl & CTRL_SBK != 0) && (old_ctrl & CTRL_SBK == 0) {
                    if let Some(transport) = &self.transport_ptr {
                        let now = virtmcu_qom::timer::qemu_clock_get_ns_safe(
                            virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL,
                            // virtmcu-allow: new_unchecked_in_peripheral reasoning="extern C timer callback is a valid BQL entry point; eliminated when replaced by ClosureTimer in P1"
                            &unsafe { virtmcu_qom::device::BqlContext::new_unchecked() }, // virtmcu-allow: unsafe_in_peripheral reasoning="extern C timer callback is a valid BQL entry point; eliminated when replaced by ClosureTimer in P1"
                        );
                        send_lin_msg(
                            &**transport,
                            &self.tx_topic,
                            LinMessageType::Break,
                            &[],
                            now as u64,
                        );
                    }
                }
                update_irqs(self.irq, &inner);
            }
            REG_DATA => {
                if inner.ctrl & CTRL_TE != 0 {
                    let byte = u8::try_from(val32 & LPUART_DATA_MASK).expect("byte truncated");
                    let was_empty = inner.tx_fifo.is_empty();
                    if inner.tx_fifo.len() < LPUART_TX_FIFO_CAP {
                        inner.tx_fifo.push_back(byte);
                    }

                    inner.stat &= !(STAT_TC | STAT_TDRE);
                    update_irqs(self.irq, &inner);

                    if was_empty {
                        let now = virtmcu_qom::timer::qemu_clock_get_ns_safe(
                            virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL,
                            // virtmcu-allow: new_unchecked_in_peripheral reasoning="extern C timer callback is a valid BQL entry point; eliminated when replaced by ClosureTimer in P1"
                            &unsafe { virtmcu_qom::device::BqlContext::new_unchecked() }, // virtmcu-allow: unsafe_in_peripheral reasoning="extern C timer callback is a valid BQL entry point; eliminated when replaced by ClosureTimer in P1"
                        );
                        if let Some(timer) = &self.tx_timer {
                            timer.mod_ns(now as i64 + calculate_baud_delay_ns(inner.baud));
                        }
                    }
                }
            }
            REG_MATCH => inner.match_ = val32,
            REG_MODIR => inner.modir = val32,
            REG_FIFO => inner.fifo = val32,
            REG_WATER => inner.water = val32,
            _ => {}
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

fn send_lin_msg(
    transport: &dyn virtmcu_wire::DataTransport,
    tx_topic: &str,
    msg_type: LinMessageType,
    data: &[u8],
    now: u64,
) {
    let mut fbb = flatbuffers::FlatBufferBuilder::new();
    let data_offset = fbb.create_vector(data);

    let args = LinFrameArgs { delivery_vtime_ns: now, type_: msg_type, data: Some(data_offset) };

    let frame = LinFrame::create(&mut fbb, &args);
    fbb.finish(frame, None);
    let finished_data = fbb.finished_data();

    match transport.reserve(tx_topic, finished_data.len()) {
        Ok(mut reservation) => {
            reservation.buffer_mut().copy_from_slice(finished_data);
            let _ = reservation.commit(now, 0);
        }
        Err(e) => {
            virtmcu_qom::sim_err!("LPUART: Failed to reserve transport: {:?}", e);
        }
    }
}

fn update_irqs(irq: QemuIrq, inner: &LpuartInner) {
    let mut pending = false;
    if (inner.ctrl & CTRL_TIE != 0) && (inner.stat & STAT_TDRE != 0) {
        pending = true;
    }
    if (inner.ctrl & CTRL_TCIE != 0) && (inner.stat & STAT_TC != 0) {
        pending = true;
    }
    if (inner.ctrl & CTRL_RIE != 0) && (inner.stat & STAT_RDRF != 0) {
        pending = true;
    }
    if (inner.ctrl & CTRL_ILIE != 0) && (inner.stat & STAT_IDLE != 0) {
        pending = true;
    }
    if (inner.baud & BAUD_LBKDIE != 0) && (inner.stat & STAT_LBKDIF != 0) {
        pending = true;
    }

    virtmcu_qom::ffi_call! {
        qemu_set_irq(irq, i32::from(pending));
    }
}

fn calculate_baud_delay_ns(baud_reg: u32) -> i64 {
    let sbr = baud_reg & LPUART_SBR_MASK;
    if sbr == 0 {
        return LPUART_DEFAULT_BAUD_DELAY_NS;
    }
    let osr = ((baud_reg >> LPUART_OSR_SHIFT) & LPUART_OSR_MASK) + 1;
    let baud_rate = LPUART_DEFAULT_CLOCK_HZ / (osr * sbr);
    if baud_rate == 0 {
        return LPUART_DEFAULT_BAUD_DELAY_NS;
    }
    (LPUART_NS_PER_SEC / i64::from(baud_rate)) * LPUART_BITS_PER_CHAR
}

// virtmcu-allow: extern_c_timer_cb reasoning="Pending ClosureTimer migration in P1"
extern "C" fn lpuart_tx_timer_cb(opaque: *mut core::ffi::c_void) {
    let state = virtmcu_qom::timer::opaque_to_state_const::<LpuartState>(opaque);
    let mut inner = state.inner.lock();

    if let Some(byte) = inner.tx_fifo.pop_front() {
        if let Some(transport) = &state.transport_ptr {
            let now = virtmcu_qom::timer::qemu_clock_get_ns_safe(
                virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL,
                // virtmcu-allow: new_unchecked_in_peripheral reasoning="extern C timer callback is a valid BQL entry point; eliminated when replaced by ClosureTimer in P1"
                &unsafe { virtmcu_qom::device::BqlContext::new_unchecked() }, // virtmcu-allow: unsafe_in_peripheral reasoning="extern C timer callback is a valid BQL entry point; eliminated when replaced by ClosureTimer in P1"
            );
            send_lin_msg(&**transport, &state.tx_topic, LinMessageType::Data, &[byte], now as u64);
        }
    }

    if inner.tx_fifo.is_empty() {
        inner.stat |= STAT_TC | STAT_TDRE;
        update_irqs(state.irq, &inner);
    } else {
        let now = virtmcu_qom::timer::qemu_clock_get_ns_safe(
            virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL,
            // virtmcu-allow: new_unchecked_in_peripheral reasoning="extern C timer callback is a valid BQL entry point; eliminated when replaced by ClosureTimer in P1"
            &unsafe { virtmcu_qom::device::BqlContext::new_unchecked() }, // virtmcu-allow: unsafe_in_peripheral reasoning="extern C timer callback is a valid BQL entry point; eliminated when replaced by ClosureTimer in P1"
        );
        if let Some(timer) = &state.tx_timer {
            timer.mod_ns(now as i64 + calculate_baud_delay_ns(inner.baud));
        }
    }
}

virtmcu_qom::register_peripheral!(S32K144LpuartQemu);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_s32k144_lpuart_qemu_layout() {
        assert_eq!(
            core::mem::offset_of!(S32K144LpuartQemu, parent_obj),
            0,
            "SysBusDevice must be the first field"
        );
    }
}
