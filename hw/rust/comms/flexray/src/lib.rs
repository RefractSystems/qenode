//! Virtmcu FlexRay controller with pluggable transport.
//! Restoration of known-working version from commit 1435f0c39b5.
use zenoh::Wait;

extern crate alloc;

use alloc::ffi::CString;
use alloc::sync::Arc;
use core::ffi::CStr;
use core::ffi::{c_char, c_uint, c_void};
use core::ptr;
use core::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use crossbeam_channel::{bounded, Receiver};
use flatbuffers::FlatBufferBuilder;
use virtmcu_api::flexray_generated::virtmcu::flexray::{FlexRayFrame, FlexRayFrameArgs};
use virtmcu_qom::declare_device_type;
use virtmcu_qom::memory::{
    MemoryRegion, MemoryRegionImplRange, MemoryRegionOps, MemoryRegionValidRange,
};
use virtmcu_qom::qdev::SysBusDevice;
use virtmcu_qom::qom::{Object, ObjectClass, TypeInfo};
use virtmcu_qom::timer::{qemu_clock_get_ns, QomTimer, QEMU_CLOCK_VIRTUAL};

#[repr(C)]
pub struct FlexRay {
    pub parent_obj: SysBusDevice,
    pub mmio: MemoryRegion,
    pub node_id: u32,
    pub router: *mut c_char,
    pub topic: *mut c_char,
    pub debug: bool,
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
    pub _liveliness: Option<zenoh::liveliness::LivelinessToken>,
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

    let router = if s.router.is_null() {
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
    match flexray_init_internal(s, s.node_id, router, topic, s.debug) {
        Ok(state) => {
            s.rust_state = state;
            virtmcu_qom::sim_err!("flexray_realize finished");
        }
        Err(e) => {
            virtmcu_qom::error_setg!(errp, "FlexRay: Zenoh initialization failed: {}", e);
        }
    }
}

unsafe extern "C" fn flexray_read(opaque: *mut c_void, addr: u64, _size: c_uint) -> u64 {
    let s = &mut *(opaque as *mut FlexRay);
    match addr {
        0x00 => u64::from(s.vrc),
        0x04 => u64::from(s.succ1),
        0x08 => u64::from(s.succ2),
        0x0C => u64::from(s.succ3),
        0x10..=0x38 => {
            let idx = (addr - 0x10) / 4;
            match idx {
                0 => u64::from(s.gtuc1),
                1 => u64::from(s.gtuc2),
                2 => u64::from(s.gtuc3),
                3 => u64::from(s.gtuc4),
                4 => u64::from(s.gtuc5),
                5 => u64::from(s.gtuc6),
                6 => u64::from(s.gtuc7),
                7 => u64::from(s.gtuc8),
                8 => u64::from(s.gtuc9),
                9 => u64::from(s.gtuc10),
                10 => u64::from(s.gtuc11),
                _ => 0,
            }
        }
        0x80 => u64::from(s.ccrr),
        0x84 => u64::from(s.ccsv),

        0x400 => u64::from(s.wrhs1),
        0x404 => u64::from(s.wrhs2),
        0x408 => u64::from(s.wrhs3),
        0x410..=0x4FF => {
            let idx = ((addr - 0x410) / 4) as usize;
            if idx < 64 {
                u64::from(s.wrds[idx])
            } else {
                0
            }
        }

        0x500 => u64::from(s.ibcr),

        0x600 => u64::from(s.orhs1),
        0x604 => u64::from(s.orhs2),
        0x608 => u64::from(s.orhs3),
        0x610..=0x6FF => {
            let idx = ((addr - 0x610) / 4) as usize;
            if idx < 64 {
                u64::from(s.ords[idx])
            } else {
                0
            }
        }
        0x700 => u64::from(s.obcr),
        _ => {
            if s.debug {
                virtmcu_qom::sim_warn!("flexray_read: unhandled offset 0x{:x}", addr);
            }
            0
        }
    }
}

unsafe extern "C" fn flexray_write(opaque: *mut c_void, addr: u64, data: u64, _size: c_uint) {
    let s = &mut *(opaque as *mut FlexRay);
    match addr {
        // MCR (Module Configuration Register): writing bit 0 = enable controller.
        // Per Bosch E-Ray semantics, enabling the module starts the cycle timer
        // so configured TX slots begin transmitting on the simulated bus.
        0x00 => {
            s.vrc = data as u32;
            if (data & 0x1) != 0 && !s.rust_state.is_null() {
                let state = unsafe { &*s.rust_state };
                let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };
                if let Some(cycle_timer) = &state.cycle_timer {
                    cycle_timer.mod_ns(now + 5_000_000);
                }
            }
        }
        0x04 => s.succ1 = data as u32,
        0x08 => s.succ2 = data as u32,
        0x0C => s.succ3 = data as u32,
        0x10..=0x38 => {
            let idx = (addr - 0x10) / 4;
            match idx {
                0 => s.gtuc1 = data as u32,
                1 => s.gtuc2 = data as u32,
                2 => s.gtuc3 = data as u32,
                3 => s.gtuc4 = data as u32,
                4 => s.gtuc5 = data as u32,
                5 => s.gtuc6 = data as u32,
                6 => s.gtuc7 = data as u32,
                7 => s.gtuc8 = data as u32,
                8 => s.gtuc9 = data as u32,
                9 => s.gtuc10 = data as u32,
                10 => s.gtuc11 = data as u32,
                _ => {}
            }
        }
        0x80 => {
            s.ccrr = data as u32;
            handle_command(s, data as u32);
        }

        0x400 => s.wrhs1 = data as u32,
        0x404 => s.wrhs2 = data as u32,
        0x408 => s.wrhs3 = data as u32,
        0x410..=0x4FF => {
            let idx = ((addr - 0x410) / 4) as usize;
            if idx < 64 {
                s.wrds[idx] = data as u32;
            }
        }

        0x500 => {
            s.ibcr = data as u32;
            let slot_idx = (data & 0x7F) as usize;
            virtmcu_qom::sim_err!("FlexRay: IBCR write slot={}, wrhs1={}", slot_idx, s.wrhs1);
            if slot_idx < 128 {
                s.msg_ram_headers[slot_idx].frame_id = s.wrhs1 as u16;
                s.msg_ram_headers[slot_idx].config = s.wrhs2;
                // Copy WRDS to msg_ram_data
                let offset = slot_idx * 64;
                for i in 0..64 {
                    if offset + i * 4 + 3 < 8192 {
                        let word = s.wrds[i];
                        s.msg_ram_data[offset + i * 4] = (word & 0xFF) as u8;
                        s.msg_ram_data[offset + i * 4 + 1] = ((word >> 8) & 0xFF) as u8;
                        s.msg_ram_data[offset + i * 4 + 2] = ((word >> 16) & 0xFF) as u8;
                        s.msg_ram_data[offset + i * 4 + 3] = ((word >> 24) & 0xFF) as u8;
                    }
                }
            }
        }

        0x700 => {
            s.obcr = data as u32;
            let slot_idx = (data & 0x7F) as usize;
            if slot_idx < 128 {
                s.orhs1 = u32::from(s.msg_ram_headers[slot_idx].frame_id);
                s.orhs2 = s.msg_ram_headers[slot_idx].config;
                s.orhs3 = 0;
                // Copy msg_ram_data to ORDS
                let offset = slot_idx * 64;
                for i in 0..64 {
                    if offset + i * 4 + 3 < 8192 {
                        let word = u32::from_le_bytes([
                            s.msg_ram_data[offset + i * 4],
                            s.msg_ram_data[offset + i * 4 + 1],
                            s.msg_ram_data[offset + i * 4 + 2],
                            s.msg_ram_data[offset + i * 4 + 3],
                        ]);
                        s.ords[i] = word;
                    }
                }
            }
        }
        _ => {
            if s.debug {
                virtmcu_qom::sim_warn!("flexray_write: unhandled offset 0x{:x}", addr);
            }
        }
    }
}

fn handle_command(s: &mut FlexRay, cmd: u32) {
    if cmd == 0x01 {
        // Coldstart
        s.ccsv = 0x2; // Normal active
    }
}

static FLEXRAY_OPS: MemoryRegionOps = MemoryRegionOps {
    read: Some(flexray_read),
    write: Some(flexray_write),
    read_with_attrs: ptr::null(),
    write_with_attrs: ptr::null(),
    endianness: virtmcu_qom::memory::DEVICE_LITTLE_ENDIAN,
    _padding1: [0; 4],
    valid: MemoryRegionValidRange {
        min_access_size: 1,
        max_access_size: 4,
        unaligned: false,
        _padding: [0; 7],
        accepts: ptr::null(),
    },
    impl_: MemoryRegionImplRange {
        min_access_size: 1,
        max_access_size: 4,
        unaligned: false,
        _padding: [0; 7],
    },
};

unsafe extern "C" fn flexray_instance_init(obj: *mut Object) {
    let s = &mut *(obj as *mut FlexRay);

    // DEBUG: Print offsets
    let base = obj as usize;
    virtmcu_qom::sim_err!(
        "FlexRay offsets: mmio={}, node_id={}, router={}, topic={}, rust_state={}, vrc={}",
        (&raw const s.mmio as usize) - base,
        (&raw const s.node_id as usize) - base,
        (&raw const s.router as usize) - base,
        (&raw const s.topic as usize) - base,
        (&raw const s.rust_state as usize) - base,
        (&raw const s.vrc as usize) - base
    );

    s.vrc = 0x00000001;
    s.ccsv = 0x0;
    virtmcu_qom::sim_err!("flexray_instance_init: initializing msg_ram");
    unsafe {
        ptr::write(&mut s.msg_ram_headers, [FlexRayMsgHeader::default(); 128]);
        ptr::write_bytes(s.msg_ram_data.as_mut_ptr(), 0, 8192);
    }

    virtmcu_qom::sim_err!("flexray_instance_init: initializing memory region");
    virtmcu_qom::memory::memory_region_init_io(
        &raw mut s.mmio,
        obj,
        &raw const FLEXRAY_OPS as *const _,
        obj as *mut c_void,
        c"flexray".as_ptr(),
        0x4000,
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
    virtmcu_qom::device_class_set_props!(dc, FLEXRAY_PROPS);
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
    virtmcu_qom::sim_err!("flexray_rx_timer_cb fired at {}", now);

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
                    virtmcu_qom::sim_err!(
                        "FlexRay RX: Matched frame_id={} in slot {}",
                        packet.frame_id,
                        i
                    );
                    let data_word = if packet.data.len() >= 4 {
                        u32::from_le_bytes(packet.data[0..4].try_into().unwrap())
                    } else {
                        0
                    };
                    virtmcu_qom::sim_err!("FlexRay RX: Updating wrhs3 and wrds[0]");
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
    router: Option<String>,
    topic: String,
    debug: bool,
) -> Result<*mut FlexRayState, String> {
    let (tx, rx) = bounded::<OrderedFlexRayPacket>(100);

    let router_cstring = router
        .as_deref()
        .map(|r| CString::new(r).expect("router endpoint must not contain interior NUL"));
    let router_ptr = router_cstring.as_ref().map_or(ptr::null(), |c| c.as_ptr());

    let session =
        unsafe { transport_zenoh::get_or_init_session(router_ptr).map_err(|e| e.to_string())? };
    let transport: Arc<dyn virtmcu_api::DataTransport> =
        Arc::new(transport_zenoh::ZenohDataTransport::new(Arc::clone(&session)));

    let liveliness =
        session.liveliness().declare_token(format!("sim/flexray/liveliness/{node_id}")).wait().ok();
    let mut state = Box::new(FlexRayState {
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
    });

    let rx_timer =
        unsafe { QomTimer::new(QEMU_CLOCK_VIRTUAL, flexray_rx_timer_cb, s_ptr as *mut c_void) };

    let rx_timer_clone = Arc::new(rx_timer);

    let sub_callback = {
        let tx = tx.clone();
        let rx_timer_clone = Arc::clone(&rx_timer_clone);
        move |payload: &[u8]| {
            virtmcu_qom::sim_err!("FlexRay RX: received {} bytes", payload.len());
            let frame = flatbuffers::root::<FlexRayFrame>(payload).unwrap();
            virtmcu_qom::sim_err!(
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
    cycle_timer.mod_ns(now + 5_000_000);
    state.cycle_timer = Some(cycle_timer);

    Ok(Box::into_raw(state))
}

extern "C" fn flexray_cycle_timer_cb(opaque: *mut core::ffi::c_void) {
    let s_ptr = opaque as *mut FlexRay;
    let s = unsafe { &mut *s_ptr };
    let state = unsafe { &*s.rust_state };

    let cycle = state.current_cycle.fetch_add(1, AtomicOrdering::SeqCst);
    virtmcu_qom::sim_err!("flexray_cycle_timer_cb fired: cycle={}", cycle);

    // Send TX frames for configured slots
    let mut sent_count = 0;
    for i in 0..128 {
        let header = &s.msg_ram_headers[i];
        if header.frame_id != 0 {
            flexray_send_frame(s, i, header.frame_id);
            sent_count += 1;
        }
    }
    virtmcu_qom::sim_err!("flexray_cycle_timer_cb sent {} frames", sent_count);

    // Schedule next cycle
    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };
    if let Some(timer) = &state.cycle_timer {
        timer.mod_ns(now + 5_000_000);
    }
}

fn flexray_send_frame(s: &mut FlexRay, slot: usize, frame_id: u16) {
    let state = unsafe { &*s.rust_state };
    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };

    let mut builder = FlatBufferBuilder::new();
    let offset = slot * 64;
    let data = &s.msg_ram_data[offset..offset + 64];
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
