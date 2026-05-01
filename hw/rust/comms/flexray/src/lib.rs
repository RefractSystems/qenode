//! Virtmcu FlexRay controller with pluggable transport.

extern crate alloc;

use alloc::boxed::Box;
use alloc::collections::BinaryHeap;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cmp::Ordering;
use core::ffi::{c_char, c_uint, c_void, CStr};
use core::ptr;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering as AtomicOrdering};
use crossbeam_channel::{bounded, Receiver};
use flatbuffers::FlatBufferBuilder;
use virtmcu_api::flexray_generated::virtmcu::flexray::{FlexRayFrame, FlexRayFrameArgs};

use virtmcu_qom::memory::{
    MemoryRegion, MemoryRegionImplRange, MemoryRegionOps, MemoryRegionValidRange,
};
use virtmcu_qom::qdev::SysBusDevice;
use virtmcu_qom::qom::{Object, ObjectClass, TypeInfo};
use virtmcu_qom::sync::{BqlGuarded, SafeSubscription};
use virtmcu_qom::timer::{qemu_clock_get_ns, QomTimer, QEMU_CLOCK_VIRTUAL};
use virtmcu_qom::{
    declare_device_type, define_prop_string, define_prop_uint32, define_properties, device_class,
};

#[repr(C)]
pub struct VirtmcuFlexRay {
    pub parent_obj: SysBusDevice,
    pub mmio: MemoryRegion,

    /* Properties */
    pub node_id: u32,
    pub transport: *mut c_char,
    pub router: *mut c_char,
    pub topic: *mut c_char,

    /* Registers */
    pub rust_state: *mut VirtmcuFlexRayState,
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

    pub msg_ram_headers: [FlexRayMsgHeader; 128],
    pub msg_ram_data: [u8; 8192],
}

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

pub struct VirtmcuFlexRayState {
    parent: *mut VirtmcuFlexRay,
    transport: Arc<dyn virtmcu_api::DataTransport>,
    node_id: u32,
    topic: String,
    subscription: Option<SafeSubscription>,
    rx_timer: Option<Arc<QomTimer>>,
    cycle_timer: Option<QomTimer>,
    rx_receiver: Receiver<OrderedFlexRayPacket>,
    local_heap: BqlGuarded<BinaryHeap<OrderedFlexRayPacket>>,
    earliest_vtime: Arc<AtomicU64>,
    current_cycle: Arc<AtomicUsize>,
}

impl PartialEq for OrderedFlexRayPacket {
    fn eq(&self, other: &Self) -> bool {
        self.vtime == other.vtime
    }
}
impl Eq for OrderedFlexRayPacket {}
impl PartialOrd for OrderedFlexRayPacket {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for OrderedFlexRayPacket {
    fn cmp(&self, other: &Self) -> Ordering {
        other.vtime.cmp(&self.vtime)
    }
}

/// # Safety
/// Called by QEMU during device realization. `dev` must be a valid pointer to `VirtmcuFlexRay`.
#[no_mangle]
pub unsafe extern "C" fn flexray_realize(dev: *mut c_void, _errp: *mut *mut c_void) {
    let s = unsafe { &mut *(dev as *mut VirtmcuFlexRay) };

    let router_str = if s.router.is_null() {
        None
    } else {
        Some(unsafe { CStr::from_ptr(s.router).to_string_lossy().into_owned() })
    };

    let transport_name = if s.transport.is_null() {
        "zenoh".to_owned()
    } else {
        unsafe { CStr::from_ptr(s.transport).to_string_lossy().into_owned() }
    };

    let topic = if s.topic.is_null() {
        "sim/flexray/frame".to_owned()
    } else {
        unsafe { CStr::from_ptr(s.topic).to_string_lossy().into_owned() }
    };

    s.rust_state = flexray_init_internal(s, s.node_id, transport_name, router_str, topic);
}

/// # Safety
/// Called by QEMU on MMIO read. `opaque` must be a valid pointer to `VirtmcuFlexRay`.
#[no_mangle]
pub unsafe extern "C" fn flexray_read(opaque: *mut c_void, addr: u64, _size: c_uint) -> u64 {
    let s = unsafe { &mut *(opaque as *mut VirtmcuFlexRay) };
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
        _ => 0,
    }
}

/// # Safety
/// Called by QEMU on MMIO write. `opaque` must be a valid pointer to `VirtmcuFlexRay`.
#[no_mangle]
pub unsafe extern "C" fn flexray_write(opaque: *mut c_void, addr: u64, data: u64, _size: c_uint) {
    let s = unsafe { &mut *(opaque as *mut VirtmcuFlexRay) };
    match addr {
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
            handle_ibcr_write(s, data as u32);
        }

        0x600 => s.orhs1 = data as u32,
        0x604 => s.orhs2 = data as u32,
        0x608 => s.orhs3 = data as u32,
        0x610..=0x6FF => {
            let idx = ((addr - 0x610) / 4) as usize;
            if idx < 64 {
                s.ords[idx] = data as u32;
            }
        }
        0x700 => {
            s.obcr = data as u32;
            handle_obcr_write(s, data as u32);
        }
        _ => {}
    }
}

fn handle_obcr_write(s: &mut VirtmcuFlexRay, val: u32) {
    let msg_idx = (val & 0x7F) as usize;
    if msg_idx >= 128 {
        return;
    }
    let header = s.msg_ram_headers[msg_idx];
    s.orhs1 = u32::from(header.frame_id);
    s.orhs2 = u32::from(header.payload_length) << 16 | u32::from(header.cycle_count);

    let offset = msg_idx * 64;
    for i in 0..16 {
        let b0 = u32::from(s.msg_ram_data[offset + i * 4]);
        let b1 = u32::from(s.msg_ram_data[offset + i * 4 + 1]);
        let b2 = u32::from(s.msg_ram_data[offset + i * 4 + 2]);
        let b3 = u32::from(s.msg_ram_data[offset + i * 4 + 3]);
        s.ords[i] = b3 << 24 | b2 << 16 | b1 << 8 | b0;
    }
}

fn handle_ibcr_write(s: &mut VirtmcuFlexRay, val: u32) {
    let msg_idx = (val & 0x7F) as usize;
    if msg_idx >= 128 {
        return;
    }
    s.msg_ram_headers[msg_idx].frame_id = (s.wrhs1 & 0x7FF) as u16;
    s.msg_ram_headers[msg_idx].payload_length = ((s.wrhs2 >> 16) & 0x7F) as u8;
    let cycle_val = (s.wrhs2 & 0xFF) as u8;
    s.msg_ram_headers[msg_idx].cycle_count = cycle_val;

    let offset = msg_idx * 64;
    if offset + 64 <= s.msg_ram_data.len() {
        for i in 0..16 {
            let word = s.wrds[i];
            s.msg_ram_data[offset + i * 4] = (word & 0xFF) as u8;
            s.msg_ram_data[offset + i * 4 + 1] = ((word >> 8) & 0xFF) as u8;
            s.msg_ram_data[offset + i * 4 + 2] = ((word >> 16) & 0xFF) as u8;
            s.msg_ram_data[offset + i * 4 + 3] = ((word >> 24) & 0xFF) as u8;
        }
    }
}

fn handle_command(s: &mut VirtmcuFlexRay, cmd: u32) {
    match cmd & 0xF {
        0x1 => s.ccsv &= !0x3F,
        0x2 => s.ccsv = (s.ccsv & !0x3F) | 0x1,
        0x4 => {
            s.ccsv = (s.ccsv & !0x3F) | 0x3;
            if !s.rust_state.is_null() {
                let state = unsafe { &*s.rust_state };
                let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };
                if let Some(cycle_timer) = &state.cycle_timer {
                    cycle_timer.mod_ns(now + 5_000_000);
                }
            }
        }
        _ => {}
    }
}

fn flexray_send_frame(s: &mut VirtmcuFlexRay, frame_id: u16, channel: u8, data: &[u8]) {
    if s.rust_state.is_null() {
        return;
    }
    let state = unsafe { &*s.rust_state };
    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;

    let mut fbb = FlatBufferBuilder::new();
    let data_offset = fbb.create_vector(data);

    let frame = FlexRayFrame::create(
        &mut fbb,
        &FlexRayFrameArgs {
            delivery_vtime_ns: now,
            frame_id,
            cycle_count: state.current_cycle.load(AtomicOrdering::SeqCst) as u8,
            channel,
            flags: 0,
            data: Some(data_offset),
        },
    );

    fbb.finish(frame, None);
    let finished_data = fbb.finished_data();

    let tx_topic = format!("{}/{}/tx", state.topic, state.node_id);
    let _ = state.transport.publish(&tx_topic, finished_data);
}

extern "C" fn flexray_cycle_timer_cb(opaque: *mut core::ffi::c_void) {
    let state = unsafe { &*(opaque as *mut VirtmcuFlexRayState) };
    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };

    let cycle = state.current_cycle.fetch_add(1, AtomicOrdering::SeqCst) % 64;

    unsafe {
        let parent = &mut *state.parent;
        parent.ccsv = (parent.ccsv & !(0x3F << 8)) | ((cycle as u32) << 8);

        for i in 0..128 {
            let header = parent.msg_ram_headers[i];
            if header.frame_id > 0
                && (header.cycle_count as usize == cycle || header.cycle_count == 0xFF)
            {
                let offset = i * 64;
                let len = (header.payload_length as usize) * 2;
                let len = if len > 64 { 64 } else { len };
                let mut data_buf = vec![0u8; len];
                data_buf.copy_from_slice(&parent.msg_ram_data[offset..offset + len]);

                flexray_send_frame(parent, header.frame_id, 0, &data_buf);
            }
        }
    }

    if let Some(cycle_timer) = &state.cycle_timer {
        cycle_timer.mod_ns(now + 5_000_000);
    }
}

static FLEXRAY_OPS: MemoryRegionOps = MemoryRegionOps {
    read: Some(flexray_read),
    write: Some(flexray_write),
    read_with_attrs: ptr::null(),
    write_with_attrs: ptr::null(),
    endianness: 2,
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

#[no_mangle]
pub unsafe extern "C" fn flexray_instance_init(obj: *mut Object) {
    let s = unsafe { &mut *(obj as *mut VirtmcuFlexRay) };
    s.vrc = 0x00000001;
    s.ccsv = 0x0;
    s.rust_state = ptr::null_mut();
    s.msg_ram_headers = [FlexRayMsgHeader::default(); 128];
    s.msg_ram_data = [0; 8192];

    unsafe {
        virtmcu_qom::memory::memory_region_init_io(
            &raw mut s.mmio,
            obj,
            &raw const FLEXRAY_OPS,
            obj as *mut c_void,
            c"flexray".as_ptr(),
            0x1000,
        );
        virtmcu_qom::qdev::sysbus_init_mmio(&raw mut s.parent_obj, &raw mut s.mmio);
    }
}

define_properties!(
    FLEXRAY_PROPS,
    [
        define_prop_uint32!(c"node".as_ptr(), VirtmcuFlexRay, node_id, 0),
        define_prop_string!(c"transport".as_ptr(), VirtmcuFlexRay, transport),
        define_prop_string!(c"router".as_ptr(), VirtmcuFlexRay, router),
        define_prop_string!(c"topic".as_ptr(), VirtmcuFlexRay, topic),
        define_prop_uint32!(c"ccsv".as_ptr(), VirtmcuFlexRay, ccsv, 0),
        define_prop_uint32!(c"succ1".as_ptr(), VirtmcuFlexRay, succ1, 0),
        define_prop_uint32!(c"wrhs3".as_ptr(), VirtmcuFlexRay, wrhs3, 0),
    ]
);

#[no_mangle]
pub unsafe extern "C" fn flexray_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let dc = device_class!(klass);
    unsafe {
        (*dc).user_creatable = true;
        (*dc).realize = Some(flexray_realize);
    }
    virtmcu_qom::device_class_set_props!(dc, FLEXRAY_PROPS);
}

#[no_mangle]
pub unsafe extern "C" fn flexray_instance_finalize(obj: *mut Object) {
    let s = unsafe { &mut *(obj as *mut VirtmcuFlexRay) };
    if !s.rust_state.is_null() {
        let mut state = unsafe { Box::from_raw(s.rust_state) };
        state.subscription.take();
        state.rx_timer.take();
        state.cycle_timer.take();
    }
}

static FLEXRAY_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"virtmcu,flexray".as_ptr(),
    parent: virtmcu_qom::qdev::TYPE_SYS_BUS_DEVICE,
    instance_size: core::mem::size_of::<VirtmcuFlexRay>(),
    instance_align: 0,
    instance_init: Some(flexray_instance_init),
    instance_post_init: None,
    instance_finalize: Some(flexray_instance_finalize),
    abstract_: false,
    class_size: 0,
    class_init: Some(flexray_class_init),
    class_base_init: None,
    class_data: ptr::null(),
    interfaces: ptr::null(),
};

declare_device_type!(FLEXRAY_TYPE_INIT, FLEXRAY_TYPE_INFO);

extern "C" fn flexray_rx_timer_cb(opaque: *mut core::ffi::c_void) {
    let state = unsafe { &*(opaque as *mut VirtmcuFlexRayState) };
    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;

    let mut heap = state.local_heap.get_mut();
    while let Ok(packet) = state.rx_receiver.try_recv() {
        heap.push(packet);
    }

    while let Some(packet) = heap.peek() {
        if packet.vtime <= now {
            let packet = heap.pop().unwrap();
            let parent = unsafe { &mut *state.parent };
            for i in 0..128 {
                if parent.msg_ram_headers[i].frame_id == packet.frame_id {
                    let offset = i * 64;
                    let len = packet.data.len();
                    let len = if len > 64 { 64 } else { len };
                    parent.msg_ram_data[offset..offset + len].copy_from_slice(&packet.data[..len]);
                    break;
                }
            }
        } else {
            break;
        }
    }

    if let (Some(next_packet), Some(rx_timer)) = (heap.peek(), &state.rx_timer) {
        state.earliest_vtime.store(next_packet.vtime, AtomicOrdering::Release);
        rx_timer.mod_ns(next_packet.vtime as i64);
    } else {
        state.earliest_vtime.store(u64::MAX, AtomicOrdering::Release);
    }
}

fn flexray_init_internal(
    parent: *mut VirtmcuFlexRay,
    node_id: u32,
    transport_name: String,
    router: Option<String>,
    topic: String,
) -> *mut VirtmcuFlexRayState {
    let transport: Arc<dyn virtmcu_api::DataTransport> = if transport_name == "unix" {
        let path = router.unwrap_or_else(|| format!("/tmp/virtmcu-coord-{node_id}.sock"));
        match transport_unix::UnixDataTransport::new(&path) {
            Ok(t) => Arc::new(t),
            Err(_) => return ptr::null_mut(),
        }
    } else {
        let router_ptr = match &router {
            Some(s) => alloc::ffi::CString::new(s.as_bytes()).unwrap().into_raw().cast_const(),
            None => ptr::null(),
        };
        let session = unsafe { transport_zenoh::get_or_init_session(router_ptr) };
        if !router_ptr.is_null() {
            unsafe {
                let _ = alloc::ffi::CString::from_raw(router_ptr.cast_mut());
            }
        }
        match session {
            Ok(s) => Arc::new(transport_zenoh::ZenohDataTransport::new(s)),
            Err(_) => return ptr::null_mut(),
        }
    };

    let (tx, rx) = bounded(1024);
    let local_heap = BqlGuarded::new(BinaryHeap::new());
    let earliest_vtime = Arc::new(AtomicU64::new(u64::MAX));
    let earliest_clone = Arc::clone(&earliest_vtime);

    let mut state = Box::new(VirtmcuFlexRayState {
        parent,
        transport,
        node_id,
        topic: topic.clone(),
        subscription: None,
        rx_timer: None,
        cycle_timer: None,
        rx_receiver: rx,
        local_heap,
        earliest_vtime,
        current_cycle: Arc::new(AtomicUsize::new(0)),
    });

    let state_ptr = &raw mut *state;
    let rx_timer = Arc::new(unsafe {
        QomTimer::new(QEMU_CLOCK_VIRTUAL, flexray_rx_timer_cb, state_ptr as *mut c_void)
    });
    let rx_timer_clone = Arc::clone(&rx_timer);
    let cycle_timer = unsafe {
        QomTimer::new(QEMU_CLOCK_VIRTUAL, flexray_cycle_timer_cb, state_ptr as *mut c_void)
    };

    let sub_callback: virtmcu_api::DataCallback = Box::new(move |data| {
        if let Ok(frame) =
            virtmcu_api::flexray_generated::virtmcu::flexray::root_as_flex_ray_frame(data)
        {
            let packet = OrderedFlexRayPacket {
                vtime: frame.delivery_vtime_ns(),
                frame_id: frame.frame_id(),
                cycle_count: frame.cycle_count(),
                channel: frame.channel(),
                flags: frame.flags(),
                data: frame.data().map(|d| d.bytes().to_vec()).unwrap_or_default(),
            };
            let _ = tx.send(packet);
            let current_earliest = earliest_clone.load(AtomicOrdering::Acquire);
            if frame.delivery_vtime_ns() < current_earliest {
                earliest_clone.fetch_min(frame.delivery_vtime_ns(), AtomicOrdering::Release);
                rx_timer_clone.mod_ns(frame.delivery_vtime_ns() as i64);
            }
        }
    });

    state.subscription =
        SafeSubscription::new(&*state.transport, &topic, Arc::new(AtomicU64::new(0)), sub_callback)
            .ok();
    state.rx_timer = Some(rx_timer);
    state.cycle_timer = Some(cycle_timer);

    Box::into_raw(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flexray_qom_layout() {
        assert_eq!(
            core::mem::offset_of!(VirtmcuFlexRay, parent_obj),
            0,
            "SysBusDevice must be the first field"
        );
    }

    #[test]
    fn test_packet_min_heap_ordering() {
        let mut heap = BinaryHeap::new();
        heap.push(OrderedFlexRayPacket {
            vtime: 500,
            frame_id: 1,
            cycle_count: 0,
            channel: 0,
            flags: 0,
            data: vec![],
        });
        heap.push(OrderedFlexRayPacket {
            vtime: 100,
            frame_id: 2,
            cycle_count: 0,
            channel: 0,
            flags: 0,
            data: vec![],
        });
        heap.push(OrderedFlexRayPacket {
            vtime: 300,
            frame_id: 3,
            cycle_count: 0,
            channel: 0,
            flags: 0,
            data: vec![],
        });
        assert_eq!(heap.pop().unwrap().vtime, 100);
        assert_eq!(heap.pop().unwrap().vtime, 300);
        assert_eq!(heap.pop().unwrap().vtime, 500);
    }
}
