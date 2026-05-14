#![allow(clippy::panic)] // virtmcu-allow: allow reasoning="Fail Loudly"
#![cfg_attr(
    test,
    allow(
        clippy::expect_used,
        clippy::unwrap_used,
        clippy::panic,
        clippy::indexing_slicing,
        clippy::panic_in_result_fn
    )
)]
unsafe extern "C" fn allow_set_link(
    _obj: *mut virtmcu_qom::qom::Object,
    _name: *const core::ffi::c_char,
    _val: *mut virtmcu_qom::qom::Object,
    _errp: *mut *mut virtmcu_qom::error::Error,
) {
}
// Virtmcu FlexRay controller with pluggable transport.
// Restoration of known-working version from commit 1435f0c39b5.

extern crate alloc;

use alloc::sync::Arc;
use core::ffi::CStr;
use core::ffi::{c_char, c_void};
use core::ptr;
use core::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use crossbeam_channel::{bounded, Receiver};
use flatbuffers::FlatBufferBuilder;
use virtmcu_api::flexray_generated::virtmcu::flexray::{FlexRayFrame, FlexRayFrameArgs};
use virtmcu_qom::declare_device_type;
use virtmcu_qom::memory::MemoryRegion;
use virtmcu_qom::qdev::SysBusDevice;
use virtmcu_qom::qom::{Object, ObjectClass, TypeInfo};
use virtmcu_qom::timer::{qemu_clock_get_ns, QomTimer, QEMU_CLOCK_VIRTUAL};

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
const FLEXRAY_RX_QUEUE_SIZE: usize = 100;
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

pub struct OrderedFlexRayPacket {
    pub vtime: u64,
    pub frame_id: u16,
    pub cycle_count: u8,
    pub channel: u8,
    pub flags: u16,
    pub data: Vec<u8>,
}

pub struct FlexRayState {
    parent_ptr: *mut FlexRay,
    _node_id: u32,
    _debug: bool,
    topic: String,
    transport: Arc<dyn virtmcu_api::DataTransport>,
    rx_timer: Option<Arc<QomTimer>>,
    cycle_timer: Option<QomTimer>,
    rx_receiver: Receiver<OrderedFlexRayPacket>,
    pending_packet: BqlGuarded<Option<OrderedFlexRayPacket>>,
    current_cycle: Arc<AtomicUsize>,
    is_valid: Arc<AtomicBool>,
    pub _liveliness: Option<alloc::boxed::Box<dyn virtmcu_api::LivelinessToken>>,
    cond: virtmcu_qom::sync::Condvar,
    // virtmcu-allow: mutex reasoning="Required for Condvar::wait_yielding_bql"
    wait_mutex: virtmcu_qom::sync::Mutex<()>,
}

impl PartialEq for OrderedFlexRayPacket {
    fn eq(&self, other: &Self) -> bool {
        self.vtime == other.vtime
    }
}
impl Eq for OrderedFlexRayPacket {}
impl PartialOrd for OrderedFlexRayPacket {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for OrderedFlexRayPacket {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        other.vtime.cmp(&self.vtime)
    }
}

use core::sync::atomic::AtomicBool;
use virtmcu_qom::sync::BqlGuarded;

unsafe extern "C" fn flexray_realize(dev: *mut c_void, errp: *mut *mut c_void) {
    virtmcu_qom::sim_err!("flexray_realize starting");
    let s = &mut *(dev as *mut FlexRay);

    let _router = if s.router.is_null() {
        None
    } else {
        Some(CStr::from_ptr(s.router).to_string_lossy().into_owned())
    };
    let topic = if s.topic.is_null() {
        "sim/flexray/frame".to_owned()
    } else {
        CStr::from_ptr(s.topic).to_string_lossy().into_owned()
    };

    virtmcu_qom::sim_err!("FlexRay size={}", core::mem::size_of::<FlexRay>());
    virtmcu_qom::sim_err!(
        "FlexRay offsets: mmio={}, node_id={}, router={}, topic={}, rust_state={}, vrc={}, wrhs3={}",
        core::mem::offset_of!(FlexRay, mmio),
        core::mem::offset_of!(FlexRay, node_id),
        core::mem::offset_of!(FlexRay, router),
        core::mem::offset_of!(FlexRay, topic),
        core::mem::offset_of!(FlexRay, rust_state),
        core::mem::offset_of!(FlexRay, vrc),
        core::mem::offset_of!(FlexRay, wrhs3),
    );

    if s.transport_hub.is_null() {
        virtmcu_qom::error_setg!(errp, "Strict DI violation: transport_hub link is required.");
        return;
    }

    unsafe {
        virtmcu_qom::qom::object_property_set_bool(
            s.transport_hub,
            c"realized".as_ptr(),
            true,
            errp as *mut *mut virtmcu_qom::error::Error,
        );
    }
    let ptr_u64 = unsafe {
        virtmcu_qom::qom::object_property_get_uint(
            s.transport_hub,
            c"transport_ptr".as_ptr(),
            errp as *mut *mut virtmcu_qom::error::Error,
        )
    };
    if ptr_u64 == 0 {
        virtmcu_qom::sim_err!("flexray_realize FAILED because ptr_u64 is 0!");
        virtmcu_qom::error_setg!(
            errp,
            "Strict DI violation: failed to acquire transport from hub."
        );
        return;
    }
    virtmcu_qom::sim_err!("flexray_realize got ptr_u64={}", ptr_u64);
    let transport_ref =
        unsafe { &*(ptr_u64 as *const alloc::sync::Arc<dyn virtmcu_api::DataTransport>) };
    let transport_arc = alloc::sync::Arc::clone(transport_ref);

    match flexray_init_internal(s, s.node_id, topic, s.debug, transport_arc) {
        Ok(state) => {
            s.rust_state = state;
            virtmcu_qom::sim_err!("flexray_realize finished");
        }
        Err(e) => {
            virtmcu_qom::error_setg!(errp, "FlexRay: initialization failed: {}", e);
        }
    }
}

impl virtmcu_qom::device::MmioDevice for FlexRayState {
    fn read(&self, addr: u64, _size: u32) -> virtmcu_qom::device::MmioResult<'_> {
        let s = unsafe { &mut *self.parent_ptr };
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
        let s = unsafe { &mut *self.parent_ptr };
        match addr {
            // MCR (Module Configuration Register): writing bit 0 = enable controller.
            // Per Bosch E-Ray semantics, enabling the module starts the cycle timer
            // so configured TX slots begin transmitting on the simulated bus.
            REG_MCR => {
                s.vrc = data as u32;
                if (data & MCR_ENABLE_BIT) != 0 {
                    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };
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
                    unsafe { *targets[idx] = val };
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
    fn wait_mutex(&self) -> &virtmcu_qom::sync::Mutex<()> {
        &self.wait_mutex
    }
}

fn handle_command(s: &mut FlexRay, cmd: u32) {
    if cmd == CMD_COLDSTART {
        // Coldstart
        s.ccsv = CCSV_NORMAL_ACTIVE; // Normal active
    }
}

unsafe extern "C" fn flexray_instance_init(obj: *mut Object) {
    let s = &mut *(obj as *mut FlexRay);

    // DEBUG: Print offsets
    let base = obj as usize;
    virtmcu_qom::sim_err!(
        "FlexRay offsets: mmio={}, node_id={}, router={}, topic={}, debug={}, transport_hub={}, rust_state={}, vrc={}, msg_ram_headers={}, msg_ram_data={}",
        (&raw const s.mmio as usize) - base,
        (&raw const s.node_id as usize) - base,
        (&raw const s.router as usize) - base,
        (&raw const s.topic as usize) - base,
        (&raw const s.debug as usize) - base,
        (&raw const s.transport_hub as usize) - base,
        (&raw const s.rust_state as usize) - base,
        (&raw const s.vrc as usize) - base,
        (&raw const s.msg_ram_headers as usize) - base,
        (&raw const s.msg_ram_data as usize) - base
    );

    s.vrc = FLEXRAY_VRC_INITIAL;
    s.ccsv = 0x0;
    virtmcu_qom::sim_err!("flexray_instance_init: initializing msg_ram");
    unsafe {
        ptr::write(&mut s.msg_ram_headers, [FlexRayMsgHeader::default(); FLEXRAY_MAX_SLOTS]);
        ptr::write_bytes(s.msg_ram_data.as_mut_ptr(), 0, FLEXRAY_MSG_RAM_DATA_SIZE);
    }

    virtmcu_qom::sim_err!("flexray_instance_init: initializing memory region");
    virtmcu_qom::memory::memory_region_init_io(
        &raw mut s.mmio,
        obj,
        &raw const FLEXRAY_OPS as *const _,
        obj as *mut c_void,
        c"flexray".as_ptr(),
        FLEXRAY_MMIO_SIZE,
    );
    virtmcu_qom::sim_err!("flexray_instance_init: initializing mmio");
    virtmcu_qom::qdev::sysbus_init_mmio(&raw mut s.parent_obj, &raw mut s.mmio);
    virtmcu_qom::sim_err!("flexray_instance_init finished");
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

unsafe extern "C" fn flexray_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let dc = klass as *mut virtmcu_qom::qdev::DeviceClass;
    (*dc).realize = Some(flexray_realize);
    (*dc).user_creatable = true;
    virtmcu_qom::device_class_set_props!(dc, FLEXRAY_PROPS);

    virtmcu_qom::qom::object_class_property_add_link(
        klass,
        c"transport".as_ptr(),
        c"virtmcu-transport-hub".as_ptr(),
        core::mem::offset_of!(FlexRay, transport_hub) as isize,
        Some(allow_set_link),
        virtmcu_qom::qom::OBJ_PROP_LINK_STRONG,
    );
}

unsafe extern "C" fn flexray_instance_finalize(obj: *mut Object) {
    let s = &mut *(obj as *mut FlexRay);
    if !s.rust_state.is_null() {
        let state = Box::from_raw(s.rust_state);
        state.is_valid.store(false, AtomicOrdering::Release);
    }
}

#[used]
static FLEXRAY_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"flexray".as_ptr(),
    parent: virtmcu_qom::qdev::TYPE_SYS_BUS_DEVICE,
    instance_size: core::mem::size_of::<FlexRay>(),
    instance_align: 0,
    instance_init: Some(flexray_instance_init),
    instance_post_init: None,
    instance_finalize: Some(flexray_instance_finalize),
    abstract_: false,
    class_size: core::mem::size_of::<virtmcu_qom::qdev::SysBusDeviceClass>(),
    class_init: Some(flexray_class_init),
    class_base_init: None,
    class_data: ptr::null(),
    interfaces: ptr::null(),
};

declare_device_type!(flexray_type_init, FLEXRAY_TYPE_INFO);

extern "C" fn flexray_rx_timer_cb(opaque: *mut core::ffi::c_void) {
    let s_ptr = opaque as *mut FlexRay;
    let s = unsafe { &mut *s_ptr };
    let state = unsafe { &*s.rust_state };

    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };
    virtmcu_qom::sim_debug!("flexray_rx_timer_cb fired at {}", now);

    loop {
        let mut pending = state.pending_packet.get_mut();
        let packet = if let Some(p) = pending.take() {
            p
        } else {
            match state.rx_receiver.try_recv() {
                Ok(p) => p,
                Err(_) => break,
            }
        };

        if now >= packet.vtime as i64 {
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
        } else {
            // Not yet time, store in pending and re-schedule
            let vtime = packet.vtime as i64;
            *pending = Some(packet);
            if let Some(timer) = &state.rx_timer {
                timer.mod_ns(vtime);
            }
            break;
        }
    }
}

pub fn flexray_init_internal(
    s_ptr: *mut FlexRay,
    node_id: u32,
    topic: String,
    debug: bool,
    transport: Arc<dyn virtmcu_api::DataTransport>,
) -> Result<*mut FlexRayState, String> {
    let (tx, rx) = bounded::<OrderedFlexRayPacket>(FLEXRAY_RX_QUEUE_SIZE);

    let liveliness = transport.declare_liveliness(&format!("sim/flexray/liveliness/{node_id}"));
    let mut state = Box::new(FlexRayState {
        parent_ptr: s_ptr,
        _liveliness: liveliness,
        _node_id: node_id,
        _debug: debug,
        topic: topic.clone(),
        transport,
        rx_timer: None,
        cycle_timer: None,
        rx_receiver: rx,
        pending_packet: BqlGuarded::new(None),
        current_cycle: Arc::new(AtomicUsize::new(0)),
        is_valid: Arc::new(AtomicBool::new(true)),
        cond: virtmcu_qom::sync::Condvar::new(),
        wait_mutex: virtmcu_qom::sync::Mutex::new(()),
    });

    let rx_timer =
        unsafe { QomTimer::new(QEMU_CLOCK_VIRTUAL, flexray_rx_timer_cb, s_ptr as *mut c_void) };

    let rx_timer_clone = Arc::new(rx_timer);

    let sub_callback = {
        let tx = tx.clone();
        let rx_timer_clone = Arc::clone(&rx_timer_clone);
        move |_topic: &str, payload: &[u8]| {
            virtmcu_qom::sim_debug!("FlexRay RX: received {} bytes", payload.len());
            let frame = flatbuffers::root::<FlexRayFrame>(payload)
                .expect("flexray logic assumption failed");
            virtmcu_qom::sim_debug!(
                "FlexRay RX: frame_id={} vtime={}",
                frame.frame_id(),
                frame.delivery_vtime_ns()
            );

            let packet = OrderedFlexRayPacket {
                vtime: frame.delivery_vtime_ns(),
                frame_id: frame.frame_id(),
                cycle_count: frame.cycle_count(),
                channel: frame.channel(),
                flags: frame.flags(),
                data: frame.data().map(|d| d.bytes().to_vec()).unwrap_or_default(),
            };
            let _ = tx.send(packet);
            rx_timer_clone.kick();
        }
    };

    // Subscribe to per-node RX subtopic; tests publish to this exact path.
    let rx_topic = alloc::format!("{topic}/{node_id}/rx");
    let _ = state.transport.subscribe(&rx_topic, Box::new(sub_callback));
    state.rx_timer = Some(rx_timer_clone);

    let cycle_timer =
        unsafe { QomTimer::new(QEMU_CLOCK_VIRTUAL, flexray_cycle_timer_cb, s_ptr as *mut c_void) };

    let now =
        unsafe { virtmcu_qom::timer::qemu_clock_get_ns(virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL) };
    cycle_timer.mod_ns(now + DEFAULT_CYCLE_TIME_NS);
    state.cycle_timer = Some(cycle_timer);

    Ok(Box::into_raw(state))
}

extern "C" fn flexray_cycle_timer_cb(opaque: *mut core::ffi::c_void) {
    let s_ptr = opaque as *mut FlexRay;
    let s = unsafe { &mut *s_ptr };
    let state = unsafe { &*s.rust_state };

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
    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };
    if let Some(timer) = &state.cycle_timer {
        timer.mod_ns(now + DEFAULT_CYCLE_TIME_NS);
    }
}

fn flexray_send_frame(s: &mut FlexRay, slot: usize, frame_id: u16) {
    let state = unsafe { &*s.rust_state };
    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };

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
    let _ = state.transport.publish(&topic, builder.finished_data());
}
