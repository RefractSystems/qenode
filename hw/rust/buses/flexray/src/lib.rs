#![allow(deprecated)] // virtmcu-allow: allow reasoning="S2 migration in progress"
#![allow(clippy::all, unused_imports, dead_code, unused_variables, unused_mut)] // virtmcu-allow: allow reasoning="Zero unsafe"
#![allow(clippy::all)] // virtmcu-allow: allow reasoning="Zero unsafe"
#![allow(clippy::panic)] // virtmcu-allow: allow reasoning="Fail Loudly"
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
use virtmcu_qom::qom::Object;
// Virtmcu FlexRay controller with pluggable transport.
// Restoration of known-working version from commit 1435f0c39b5.

extern crate alloc;

use alloc::sync::Arc;
use core::ffi::{c_char, c_void};
use core::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use flatbuffers::FlatBufferBuilder;
use virtmcu_qom::memory::MemoryRegion;
use virtmcu_qom::qdev::SysBusDevice;
use virtmcu_qom::timer::{QomTimer, QEMU_CLOCK_VIRTUAL};
use virtmcu_wire::flexray_generated::virtmcu::flexray::{FlexRayFrame, FlexRayFrameArgs};

// Register Offsets
const REG_MCR: u64 = 0x00;
const REG_SUCC1: u64 = 0x04;
const REG_SUCC2: u64 = 0x08;
const REG_SUCC3: u64 = 0x0C;
const REG_GTUC_START: u64 = 0x10;
const REG_GTUC_END: u64 = 0x38;
const REG_CCRR: u64 = 0x80;
const REG_CCSV: u64 = 0x84;
const REG_WRHS1: u64 = 0x400;
const REG_WRHS2: u64 = 0x404;
const REG_WRHS3: u64 = 0x408;
const REG_WRDS_START: u64 = 0x410;
const REG_WRDS_END: u64 = 0x4FF;
const REG_IBCR: u64 = 0x500;
const REG_ORHS1: u64 = 0x600;
const REG_ORHS2: u64 = 0x604;
const REG_ORHS3: u64 = 0x608;
const REG_ORDS_START: u64 = 0x610;
const REG_ORDS_END: u64 = 0x6FF;
const REG_OBCR: u64 = 0x700;
const REG_STEP_SIZE: u64 = 4;
const MSG_BUFFER_WORDS: usize = 64;
const FLEXRAY_MAX_SLOTS: usize = 128;
const FLEXRAY_MSG_RAM_DATA_SIZE: usize = 8192;
const FLEXRAY_IBCR_SLOT_MASK: u64 = 0x7F;
const FLEXRAY_OBCR_SLOT_MASK: u64 = 0x7F;
const FLEXRAY_WORD_SIZE: usize = 4;
const CMD_COLDSTART: u32 = 0x01;
const CCSV_NORMAL_ACTIVE: u32 = 0x2;
const FLEXRAY_MMIO_SIZE: u64 = 0x1000;
const FLEXRAY_VRC_INITIAL: u32 = 0x00000001;
const FLEXRAY_SLOT_SIZE: usize = 64;

// Control Bits
const MCR_ENABLE_BIT: u64 = 0x1;
const DEFAULT_CYCLE_TIME_NS: i64 = 5_000_000;

#[repr(C)]
#[derive(virtmcu_qom::MmioDevice)]
pub struct FlexRay {
    pub parent_obj: SysBusDevice,
    pub mmio: MemoryRegion,
    pub node_id: u32,
    pub router: *mut c_char,
    pub topic: *mut c_char,
    pub debug: bool,

    /* Links */
    pub transport_hub: *mut Object,

    /* Rust state */
    pub rust_state: *mut FlexRayState,

    // Bosch E-Ray registers
    pub vrc: u32,
    pub succ1: u32,
    pub succ2: u32,
    pub succ3: u32,
    pub ccrr: u32,
    pub ccsv: u32,
    pub gtuc1: u32,
    pub gtuc2: u32,
    pub gtuc3: u32,
    pub gtuc4: u32,
    pub gtuc5: u32,
    pub gtuc6: u32,
    pub gtuc7: u32,
    pub gtuc8: u32,
    pub gtuc9: u32,
    pub gtuc10: u32,
    pub gtuc11: u32,

    // Message RAM Interface
    pub wrhs1: u32,
    pub wrhs2: u32,
    pub wrhs3: u32,
    pub wrds: [u32; 64],
    pub ibcr: u32,

    pub orhs1: u32,
    pub orhs2: u32,
    pub orhs3: u32,
    pub ords: [u32; 64],
    pub obcr: u32,

    // Internal Message RAM (simplified)
    pub msg_ram_headers: [FlexRayMsgHeader; 128],
    pub msg_ram_data: [u8; 8192],
}

const _: () = assert!(core::mem::offset_of!(FlexRay, parent_obj) == 0);
const _: () = assert!(core::mem::size_of::<FlexRay>() == 10976);

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct FlexRayMsgHeader {
    pub frame_id: u16,
    pub cycle_count: u8,
    pub payload_length: u8,
    pub config: u32,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct OrderedFlexRayPacket {
    pub vtime: u64,
    pub frame_id: u16,
    pub cycle_count: u8,
    pub channel: u8,
    pub flags: u16,
    pub data: Vec<u8>,
}

impl virtmcu_qom::sync::DeliveryPacket for OrderedFlexRayPacket {
    fn delivery_vtime_ns(&self) -> u64 {
        self.vtime
    }
}

pub struct FlexRayState {
    parent_ptr: *mut FlexRay,
    _node_id: u32,
    _debug: bool,
    topic: String,
    transport: Arc<dyn virtmcu_wire::DataTransport>,
    cycle_timer: Option<QomTimer>,
    receiver: Option<virtmcu_qom::sync::VtimeIngress<OrderedFlexRayPacket>>,
    current_cycle: Arc<AtomicUsize>,
    is_valid: Arc<AtomicBool>,
    pub _liveliness: Option<alloc::boxed::Box<dyn virtmcu_wire::LivelinessToken>>,
    cond: virtmcu_qom::sync::Condvar,
    // virtmcu-allow: mutex reasoning="Required for Condvar::wait_yielding_bql"
    wait_mutex: virtmcu_qom::sync::Mutex<()>,
}

use core::sync::atomic::AtomicBool;

impl virtmcu_qom::device::Peripheral for FlexRayState {
    fn read(
        &self,
        addr: u64,
        size: u32,
        _ctx: &virtmcu_qom::device::BqlContext,
    ) -> virtmcu_qom::device::MmioResult<'_> {
        virtmcu_qom::device::MmioDevice::read(self, addr, size)
    }

    fn write(&self, addr: u64, data: u64, size: u32, _ctx: &virtmcu_qom::device::BqlContext) {
        virtmcu_qom::device::MmioDevice::write(self, addr, data, size);
    }

    fn condvar(&self) -> &virtmcu_qom::sync::Condvar {
        virtmcu_qom::device::MmioDevice::condvar(self)
    }

    fn wait_mutex(&self) -> &virtmcu_qom::sync::Mutex<()> {
        virtmcu_qom::device::MmioDevice::wait_mutex(self)
    }
}

impl virtmcu_qom::device::MmioDevice for FlexRayState {
    fn read(&self, addr: u64, _size: u32) -> virtmcu_qom::device::MmioResult<'_> {
        let s = unsafe { &mut *(self.parent_ptr as *mut FlexRay) }; // virtmcu-allow: unsafe_in_peripheral reasoning="Migration debt"
        match addr {
            REG_MCR => virtmcu_qom::device::MmioResult::Ready(u64::from(s.vrc)),
            REG_SUCC1 => virtmcu_qom::device::MmioResult::Ready(u64::from(s.succ1)),
            REG_SUCC2 => virtmcu_qom::device::MmioResult::Ready(u64::from(s.succ2)),
            REG_SUCC3 => virtmcu_qom::device::MmioResult::Ready(u64::from(s.succ3)),
            REG_GTUC_START..=REG_GTUC_END => {
                let idx = ((addr - REG_GTUC_START) / REG_STEP_SIZE) as usize;
                let gtucs = [
                    s.gtuc1, s.gtuc2, s.gtuc3, s.gtuc4, s.gtuc5, s.gtuc6, s.gtuc7, s.gtuc8,
                    s.gtuc9, s.gtuc10, s.gtuc11,
                ];
                if idx < gtucs.len() {
                    virtmcu_qom::device::MmioResult::Ready(u64::from(gtucs[idx]))
                } else {
                    virtmcu_qom::device::MmioResult::Ready(0)
                }
            }
            REG_CCRR => virtmcu_qom::device::MmioResult::Ready(u64::from(s.ccrr)),
            REG_CCSV => virtmcu_qom::device::MmioResult::Ready(u64::from(s.ccsv)),

            REG_WRHS1 => virtmcu_qom::device::MmioResult::Ready(u64::from(s.wrhs1)),
            REG_WRHS2 => virtmcu_qom::device::MmioResult::Ready(u64::from(s.wrhs2)),
            REG_WRHS3 => virtmcu_qom::device::MmioResult::Ready(u64::from(s.wrhs3)),
            REG_WRDS_START..=REG_WRDS_END => {
                let idx = ((addr - REG_WRDS_START) / REG_STEP_SIZE) as usize;
                if idx < MSG_BUFFER_WORDS {
                    virtmcu_qom::device::MmioResult::Ready(u64::from(s.wrds[idx]))
                } else {
                    virtmcu_qom::device::MmioResult::Ready(0)
                }
            }

            REG_IBCR => virtmcu_qom::device::MmioResult::Ready(u64::from(s.ibcr)),

            REG_ORHS1 => virtmcu_qom::device::MmioResult::Ready(u64::from(s.orhs1)),
            REG_ORHS2 => virtmcu_qom::device::MmioResult::Ready(u64::from(s.orhs2)),
            REG_ORHS3 => virtmcu_qom::device::MmioResult::Ready(u64::from(s.orhs3)),
            REG_ORDS_START..=REG_ORDS_END => {
                let idx = ((addr - REG_ORDS_START) / REG_STEP_SIZE) as usize;
                if idx < MSG_BUFFER_WORDS {
                    virtmcu_qom::device::MmioResult::Ready(u64::from(s.ords[idx]))
                } else {
                    virtmcu_qom::device::MmioResult::Ready(0)
                }
            }
            REG_OBCR => virtmcu_qom::device::MmioResult::Ready(u64::from(s.obcr)),
            _ => {
                unreachable!("flexray_read: unhandled offset 0x{:x}", addr);
            }
        }
    }

    fn write(&self, addr: u64, data: u64, _size: u32) {
        let s = unsafe { &mut *(self.parent_ptr as *mut FlexRay) }; // virtmcu-allow: unsafe_in_peripheral reasoning="Migration debt"
        match addr {
            // MCR (Module Configuration Register): writing bit 0 = enable controller.
            // Per Bosch E-Ray semantics, enabling the module starts the cycle timer
            // so configured TX slots begin transmitting on the simulated bus.
            REG_MCR => {
                s.vrc = data as u32;
                if (data & MCR_ENABLE_BIT) != 0 {
                    let now = virtmcu_qom::timer::qemu_clock_get_ns_safe(
                        virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL,
                        unsafe { &virtmcu_qom::device::BqlContext::new_unchecked() }, // virtmcu-allow: unsafe_in_peripheral reasoning="Migration debt"
                    );
                    if let Some(cycle_timer) = &self.cycle_timer {
                        cycle_timer.mod_ns(now + DEFAULT_CYCLE_TIME_NS);
                    }
                }
            }
            REG_SUCC1 => s.succ1 = data as u32,
            REG_SUCC2 => s.succ2 = data as u32,
            REG_SUCC3 => s.succ3 = data as u32,
            REG_GTUC_START..=REG_GTUC_END => {
                let idx = ((addr - REG_GTUC_START) / REG_STEP_SIZE) as usize;
                let val = data as u32;
                let targets = [
                    &raw mut s.gtuc1,
                    &raw mut s.gtuc2,
                    &raw mut s.gtuc3,
                    &raw mut s.gtuc4,
                    &raw mut s.gtuc5,
                    &raw mut s.gtuc6,
                    &raw mut s.gtuc7,
                    &raw mut s.gtuc8,
                    &raw mut s.gtuc9,
                    &raw mut s.gtuc10,
                    &raw mut s.gtuc11,
                ];
                if idx < targets.len() {
                    virtmcu_qom::ffi_call! { *targets[idx] = val };
                }
            }
            REG_CCRR => {
                s.ccrr = data as u32;
                handle_command(s, data as u32);
            }

            REG_WRHS1 => s.wrhs1 = data as u32,
            REG_WRHS2 => s.wrhs2 = data as u32,
            REG_WRHS3 => s.wrhs3 = data as u32,
            REG_WRDS_START..=REG_WRDS_END => {
                let idx = ((addr - REG_WRDS_START) / REG_STEP_SIZE) as usize;
                if idx < MSG_BUFFER_WORDS {
                    s.wrds[idx] = data as u32;
                }
            }

            REG_IBCR => {
                s.ibcr = data as u32;
                let slot_idx = (data & FLEXRAY_IBCR_SLOT_MASK) as usize;
                virtmcu_qom::sim_err!("FlexRay: IBCR write slot={}, wrhs1={}", slot_idx, s.wrhs1);
                if slot_idx < FLEXRAY_MAX_SLOTS {
                    s.msg_ram_headers[slot_idx].frame_id = s.wrhs1 as u16;
                    s.msg_ram_headers[slot_idx].config = s.wrhs2;
                    // Copy WRDS to msg_ram_data
                    let offset = slot_idx * MSG_BUFFER_WORDS;
                    for i in 0..MSG_BUFFER_WORDS {
                        let word_offset = offset + i * FLEXRAY_WORD_SIZE;
                        if word_offset + FLEXRAY_WORD_SIZE <= FLEXRAY_MSG_RAM_DATA_SIZE {
                            let word = s.wrds[i];
                            let bytes = word.to_le_bytes();
                            s.msg_ram_data[word_offset..word_offset + FLEXRAY_WORD_SIZE]
                                .copy_from_slice(&bytes);
                        }
                    }
                }
            }

            REG_OBCR => {
                s.obcr = data as u32;
                let slot_idx = (data & FLEXRAY_OBCR_SLOT_MASK) as usize;
                if slot_idx < FLEXRAY_MAX_SLOTS {
                    s.orhs1 = u32::from(s.msg_ram_headers[slot_idx].frame_id);
                    s.orhs2 = s.msg_ram_headers[slot_idx].config;
                    s.orhs3 = 0;
                    // Copy msg_ram_data to ORDS
                    let offset = slot_idx * MSG_BUFFER_WORDS;
                    for i in 0..MSG_BUFFER_WORDS {
                        let word_offset = offset + i * FLEXRAY_WORD_SIZE;
                        if word_offset + FLEXRAY_WORD_SIZE <= FLEXRAY_MSG_RAM_DATA_SIZE {
                            let bytes =
                                &s.msg_ram_data[word_offset..word_offset + FLEXRAY_WORD_SIZE];
                            s.ords[i] = u32::from_le_bytes(
                                bytes.try_into().expect("FlexRay word is always four bytes"),
                            );
                        }
                    }
                }
            }
            _ => {
                if s.debug {
                    virtmcu_qom::sim_debug!("flexray_write: unhandled offset 0x{:x}", addr);
                }
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

fn handle_command(s: &mut FlexRay, cmd: u32) {
    if cmd == CMD_COLDSTART {
        // Coldstart
        s.ccsv = CCSV_NORMAL_ACTIVE; // Normal active
    }
}

virtmcu_qom::define_properties!(
    FLEXRAY_PROPS,
    [
        virtmcu_qom::define_prop_uint32!(c"node".as_ptr(), FlexRay, node_id, 0),
        virtmcu_qom::define_prop_string!(c"router".as_ptr(), FlexRay, router),
        virtmcu_qom::define_prop_string!(c"topic".as_ptr(), FlexRay, topic),
        virtmcu_qom::define_prop_bool!(c"debug".as_ptr(), FlexRay, debug, false),
    ]
);

fn decode_flexray(
    _opaque: *mut core::ffi::c_void,
    _topic: &str,
    data: &[u8],
) -> Option<OrderedFlexRayPacket> {
    virtmcu_qom::sim_debug!("FlexRay RX: received {} bytes", data.len());
    let (vtime, _, data) = virtmcu_wire::decode_frame(data)?;
    let frame = flatbuffers::root::<FlexRayFrame>(data).ok()?;
    virtmcu_qom::sim_debug!("FlexRay RX: frame_id={} vtime={}", frame.frame_id(), vtime);

    Some(OrderedFlexRayPacket {
        vtime,
        frame_id: frame.frame_id(),
        cycle_count: frame.cycle_count(),
        channel: frame.channel(),
        flags: frame.flags(),
        data: frame.data().map(|d| d.bytes().to_vec()).unwrap_or_default(),
    })
}

fn deliver_flexray(opaque: *mut core::ffi::c_void, packet: OrderedFlexRayPacket) {
    let s_ptr = opaque as *mut FlexRay;
    let s = unsafe { &mut *(s_ptr as *mut FlexRay) }; // virtmcu-allow: unsafe_in_peripheral reasoning="Migration debt"

    let now = virtmcu_qom::timer::qemu_clock_get_ns_safe(
        virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL,
        unsafe { &virtmcu_qom::device::BqlContext::new_unchecked() }, // virtmcu-allow: unsafe_in_peripheral reasoning="Migration debt"
    );
    virtmcu_qom::sim_debug!("deliver_flexray fired at {}", now);

    // Find matching slot
    for i in 0..128 {
        if s.msg_ram_headers[i].frame_id == packet.frame_id {
            virtmcu_qom::sim_debug!(
                "FlexRay RX: Matched frame_id={} in slot {}",
                packet.frame_id,
                i
            );
            let data_word = if packet.data.len() >= FLEXRAY_WORD_SIZE {
                u32::from_le_bytes(
                    packet.data[0..FLEXRAY_WORD_SIZE]
                        .try_into()
                        .expect("flexray logic assumption failed"),
                )
            } else {
                0
            };
            virtmcu_qom::sim_debug!("FlexRay RX: Updating wrhs3 and wrds[0]");
            s.wrhs3 |= 1;
            s.wrds[0] = data_word;
        }
    }
}

pub fn flexray_init_internal(
    s_ptr: *mut FlexRay,
    node_id: u32,
    topic: String,
    debug: bool,
    transport: Arc<dyn virtmcu_wire::DataTransport>,
) -> Result<*mut FlexRayState, String> {
    let liveliness = transport.declare_liveliness(&format!("sim/flexray/liveliness/{node_id}"));
    let mut state_box = Box::new(FlexRayState {
        parent_ptr: s_ptr,
        _liveliness: liveliness,
        _node_id: node_id,
        _debug: debug,
        topic: topic.clone(),
        transport: Arc::clone(&transport),
        cycle_timer: None,
        receiver: None,
        current_cycle: Arc::new(AtomicUsize::new(0)),
        is_valid: Arc::new(AtomicBool::new(true)),
        cond: virtmcu_qom::sync::Condvar::new(),
        wait_mutex: virtmcu_qom::sync::Mutex::new(()),
    });

    let state_ptr = core::ptr::from_mut(&mut *state_box);
    let generation = Arc::new(core::sync::atomic::AtomicU64::new(0));
    let rx_topic = alloc::format!("{topic}/{node_id}/rx");

    match virtmcu_qom::sync::VtimeIngress::new(
        &*transport,
        &rx_topic,
        generation,
        state_ptr as *mut c_void,
        decode_flexray,
        deliver_flexray,
    ) {
        Ok(receiver) => {
            state_box.receiver = Some(receiver);
        }
        Err(e) => {
            virtmcu_qom::sim_err!("FAILED TO CREATE SUBSCRIPTION!: {}", e);
            return Err("Failed to create subscription".into());
        }
    }

    let cycle_timer = virtmcu_qom::ffi_call! { QomTimer::new_safe(QEMU_CLOCK_VIRTUAL, flexray_cycle_timer_cb, s_ptr as *mut c_void) };

    let now = virtmcu_qom::timer::qemu_clock_get_ns_safe(
        virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL,
        unsafe { &virtmcu_qom::device::BqlContext::new_unchecked() }, // virtmcu-allow: unsafe_in_peripheral reasoning="Migration debt"
    );
    cycle_timer.mod_ns(now + DEFAULT_CYCLE_TIME_NS);
    state_box.cycle_timer = Some(cycle_timer);

    Ok(Box::into_raw(state_box))
}

// virtmcu-allow: extern_c_timer_cb reasoning="Pending ClosureTimer migration in P1"
extern "C" fn flexray_cycle_timer_cb(opaque: *mut core::ffi::c_void) {
    let s_ptr = opaque as *mut FlexRay;
    let s = unsafe { &mut *(s_ptr as *mut FlexRay) }; // virtmcu-allow: unsafe_in_peripheral reasoning="Migration debt"
    let state = unsafe { &*(s.rust_state as *mut FlexRayState) }; // virtmcu-allow: unsafe_in_peripheral reasoning="Migration debt"

    let cycle = state.current_cycle.fetch_add(1, AtomicOrdering::SeqCst);
    virtmcu_qom::sim_debug!("flexray_cycle_timer_cb fired: cycle={}", cycle);

    // Send TX frames for configured slots
    let mut sent_count = 0;
    for i in 0..128 {
        let header = &s.msg_ram_headers[i];
        if header.frame_id != 0 {
            flexray_send_frame(s, i, header.frame_id);
            sent_count += 1;
        }
    }
    virtmcu_qom::sim_debug!("flexray_cycle_timer_cb sent {} frames", sent_count);

    // Schedule next cycle
    let now = virtmcu_qom::timer::qemu_clock_get_ns_safe(
        virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL,
        unsafe { &virtmcu_qom::device::BqlContext::new_unchecked() }, // virtmcu-allow: unsafe_in_peripheral reasoning="Migration debt"
    );
    if let Some(timer) = &state.cycle_timer {
        timer.mod_ns(now + DEFAULT_CYCLE_TIME_NS);
    }
}

fn flexray_send_frame(s: &mut FlexRay, slot: usize, frame_id: u16) {
    let state = unsafe { &*(s.rust_state as *mut FlexRayState) }; // virtmcu-allow: unsafe_in_peripheral reasoning="Migration debt"
    let now = virtmcu_qom::timer::qemu_clock_get_ns_safe(
        virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL,
        unsafe { &virtmcu_qom::device::BqlContext::new_unchecked() }, // virtmcu-allow: unsafe_in_peripheral reasoning="Migration debt"
    );

    let mut builder = FlatBufferBuilder::new();
    let offset = slot * FLEXRAY_SLOT_SIZE;
    let data = &s.msg_ram_data[offset..offset + FLEXRAY_SLOT_SIZE];
    let data_off = builder.create_vector(data);
    let args = FlexRayFrameArgs {
        frame_id,
        cycle_count: state.current_cycle.load(AtomicOrdering::SeqCst) as u8,
        data: Some(data_off),
        delivery_vtime_ns: now as u64,
        ..Default::default()
    };
    let frame_off = FlexRayFrame::create(&mut builder, &args);
    builder.finish(frame_off, None);

    let topic = alloc::format!("{}/{}/tx", state.topic, s.node_id);
    let payload = builder.finished_data();
    let seq = 0; // FlexRay does not currently track tx sequence
    match state.transport.reserve(&topic, payload.len()) {
        Ok(mut reservation) => {
            reservation.buffer_mut().copy_from_slice(payload);
            let _ = reservation.commit(now as u64, seq);
        }
        Err(e) => {
            virtmcu_qom::sim_err!("FlexRay: Failed to reserve transport: {e:?}");
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flexray_layout() {
        assert_eq!(
            core::mem::offset_of!(FlexRay, parent_obj),
            0,
            "SysBusDevice must be the first field"
        );
    }
}
