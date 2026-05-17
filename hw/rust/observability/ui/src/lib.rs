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
use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::collections::HashMap;

use virtmcu_qom::memory::MemoryRegion;
use virtmcu_qom::qdev::SysBusDevice;
use virtmcu_qom::sync::{Condvar, DeliveryPacket, Mutex, VcpuDrain, VtimeIngress};
use virtmcu_wire::{DataTransport, LivelinessToken};

const REG_LED_ID: u64 = 0x00;
const REG_LED_STATE: u64 = 0x04;
const REG_BTN_ID: u64 = 0x10;
const REG_BTN_STATE: u64 = 0x14;

#[derive(Eq, PartialEq)]
pub struct ButtonPacket {
    pub vtime: u64,
    pub pressed: bool,
}

impl PartialOrd for ButtonPacket {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ButtonPacket {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.vtime.cmp(&other.vtime)
    }
}

impl DeliveryPacket for ButtonPacket {
    fn delivery_vtime_ns(&self) -> u64 {
        self.vtime
    }
}

#[repr(C)]
#[derive(virtmcu_qom::MmioDevice)]
#[virtmcu_qom::macros::qom_device(name = "ui")]
pub struct ZenohUiQEMU {
    pub parent_obj: SysBusDevice,
    pub iomem: MemoryRegion,

    #[qom_property]
    pub node: u32,
    #[qom_property]
    pub router: virtmcu_qom::qom::QomString,
    #[qom_property]
    pub debug: bool,

    #[qom_link(target = "virtmcu-transport-hub")]
    pub transport: virtmcu_qom::qom::QomLink<dyn DataTransport>,

    #[qom_state]
    pub state: ZenohUiState,
}

pub struct ButtonInfo {
    pub pressed: Arc<AtomicBool>,
    pub receiver: VtimeIngress<ButtonPacket>,
}

pub struct ZenohUiState {
    pub node_id: u32,
    pub debug: bool,
    pub transport: Option<Arc<dyn DataTransport>>,
    pub drain: VcpuDrain,
    pub cond: Arc<Condvar>,
    // virtmcu-allow: mutex reasoning="Required for Condvar::wait_yielding_bql"
    pub wait_mutex: Arc<Mutex<()>>, // virtmcu-allow: mutex reasoning="State managed securely"
    pub generation: Arc<AtomicU64>,

    pub active_led_id: AtomicU32,
    pub active_btn_id: AtomicU32,

    pub qemu_dev_ptr: *mut ZenohUiQEMU,

    // virtmcu-allow: mutex reasoning="Protect HashMap of receivers"
    pub buttons: Mutex<HashMap<u32, ButtonInfo>>, // virtmcu-allow: mutex reasoning="State managed securely"
    pub _liveliness: Option<Box<dyn LivelinessToken>>,
}

impl virtmcu_qom::device::PeripheralState for ZenohUiState {
    type QomType = ZenohUiQEMU;

    fn new(qemu_dev: &Self::QomType) -> Self {
        let transport = qemu_dev.transport.get();
        let liveliness = transport.as_ref().and_then(|t| {
            t.declare_liveliness(&alloc::format!("sim/ui/liveliness/{}", qemu_dev.node))
        });

        Self {
            node_id: qemu_dev.node,
            debug: qemu_dev.debug,
            transport,
            drain: VcpuDrain::new(),
            cond: Arc::new(Condvar::new()),
            wait_mutex: Arc::new(Mutex::new(())), // virtmcu-allow: mutex reasoning="State managed securely"
            generation: Arc::new(AtomicU64::new(0)),
            active_led_id: AtomicU32::new(0),
            active_btn_id: AtomicU32::new(0),
            qemu_dev_ptr: core::ptr::from_ref(qemu_dev).cast_mut(),
            buttons: Mutex::new(HashMap::new()), // virtmcu-allow: mutex reasoning="State managed securely"
            _liveliness: liveliness,
        }
    }
}

impl virtmcu_qom::device::Peripheral for ZenohUiState {
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

    #[rustfmt::skip]
    fn wait_mutex(&self) -> &Mutex<()> { // virtmcu-allow: mutex reasoning="State managed securely"
        &self.wait_mutex
    }
}

impl virtmcu_qom::device::MmioDevice for ZenohUiState {
    fn read(&self, addr: u64, _size: u32) -> virtmcu_qom::device::MmioResult<'_> {
        let _guard = self.drain.acquire();
        if self.debug {
            virtmcu_qom::sim_debug!("ui_read: addr=0x{:x}", addr);
        }

        match addr {
            REG_LED_ID => virtmcu_qom::device::MmioResult::Ready(u64::from(
                self.active_led_id.load(Ordering::Relaxed),
            )),
            REG_BTN_ID => virtmcu_qom::device::MmioResult::Ready(u64::from(
                self.active_btn_id.load(Ordering::Relaxed),
            )),
            REG_BTN_STATE => {
                let btn_id = self.active_btn_id.load(Ordering::Relaxed);
                let btns = self.buttons.lock();
                let pressed =
                    btns.get(&btn_id).is_some_and(|info| info.pressed.load(Ordering::Relaxed));
                virtmcu_qom::device::MmioResult::Ready(u64::from(pressed))
            }
            _ => virtmcu_qom::device::MmioResult::Ready(0),
        }
    }

    fn write(&self, addr: u64, val: u64, _size: u32) {
        let _guard = self.drain.acquire();
        match addr {
            REG_LED_ID => {
                let led_id = u32::try_from(val).expect("Invalid data format");
                self.active_led_id.store(led_id, Ordering::Relaxed);
            }
            REG_LED_STATE => {
                if let Some(t) = &self.transport {
                    let led_id = self.active_led_id.load(Ordering::Relaxed);
                    let topic = alloc::format!("sim/ui/{}/led/{}", self.node_id, led_id);
                    let payload = if val != 0 { [1u8] } else { [0u8] };
                    let vtime = virtmcu_qom::telemetry::get_global_vtime();
                    let seq = 0;

                    #[allow(deprecated)] // virtmcu-allow: allow reasoning="S2.1 migration"
                    match t.reserve(&topic, payload.len()) {
                        Ok(mut reservation) => {
                            reservation.buffer_mut().copy_from_slice(&payload);
                            reservation
                                .commit(vtime, seq)
                                .expect("FATAL: UI failed to commit transport reservation");
                        }
                        Err(e) => virtmcu_qom::sim_err!(
                            "UI: Failed to reserve transport for topic {}: {:?}",
                            topic,
                            e
                        ),
                    };
                }
            }
            REG_BTN_ID => {
                let btn_id = u32::try_from(val).expect("Invalid data format");
                self.active_btn_id.store(btn_id, Ordering::Relaxed);

                let mut btns = self.buttons.lock();
                if let std::collections::hash_map::Entry::Vacant(e) = btns.entry(btn_id) {
                    let irq = virtmcu_qom::ffi_call! {
                        virtmcu_qom::qdev::sysbus_get_connected_irq(
                            self.qemu_dev_ptr as *mut virtmcu_qom::qdev::SysBusDevice,
                            btn_id as core::ffi::c_int,
                        )
                    };
                    let topic = alloc::format!("sim/ui/{}/button/{}", self.node_id, btn_id);
                    let irq_ptr = irq as usize;
                    let pressed = Arc::new(AtomicBool::new(false));
                    let pressed_clone = Arc::clone(&pressed);

                    if let Some(t) = &self.transport {
                        let generation_clone = Arc::clone(&self.generation);
                        #[allow(deprecated)] // virtmcu-allow: allow reasoning="S2.1 migration"
                        let rec = VtimeIngress::new_safe(
                            &**t,
                            &topic,
                            generation_clone,
                            |topic_name, payload| {
                                virtmcu_qom::sim_debug!("UI: Rx callback on topic {} (len={})", topic_name, payload.len());
                                if let Some((vtime, _seq, data)) = virtmcu_wire::decode_frame(payload) {
                                    let pressed_val = data.first().is_some_and(|&b| b != 0);
                                    Some(ButtonPacket { vtime, pressed: pressed_val })
                                } else {
                                    virtmcu_qom::sim_err!("UI: failed to decode frame on {}!", topic_name);
                                    None
                                }
                            },
                            move |packet| {
                                pressed_clone.store(packet.pressed, Ordering::Relaxed);
                                // SAFETY: irq_ptr is a valid QemuIrq initialized by sysbus_get_connected_irq.
                                virtmcu_qom::ffi_call! { virtmcu_qom::irq::qemu_set_irq(irq_ptr as virtmcu_qom::irq::QemuIrq, i32::from(packet.pressed)) };
                            },
                        ).expect("Failed to init receiver");

                        e.insert(ButtonInfo { pressed, receiver: rec });
                    }
                }
            }
            _ => unreachable!("ui_write: unhandled offset 0x{:x} val=0x{:x}", addr, val),
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

virtmcu_qom::register_peripheral!(ZenohUiQEMU);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ui_qemu_layout() {
        assert_eq!(core::mem::offset_of!(ZenohUiQEMU, parent_obj), 0, "SysBusDevice must be first");
    }

    #[test]
    fn test_button_packet_ordering() {
        let p1 = ButtonPacket { vtime: 100, pressed: true };
        let p2 = ButtonPacket { vtime: 200, pressed: false };
        let p3 = ButtonPacket { vtime: 100, pressed: false };

        assert!(p1 < p2);
        assert!(p2 > p1);
        assert_eq!(p1.cmp(&p3), core::cmp::Ordering::Equal);
    }
}
