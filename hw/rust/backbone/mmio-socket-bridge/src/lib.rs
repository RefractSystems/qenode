//! # MMIO Socket Bridge
//!
//! Lock ordering: BQL -> SharedState Mutex -> (Condvar releases Mutex temporarily).
//! Background I/O thread never acquires BQL.
//! vCPU thread acquires BQL (held by QEMU), then locks SharedState Mutex, then
//! waits on Condvar (which releases Mutex). BQL is temporarily yielded during wait
//! via Bql::temporary_unlock().

extern crate alloc;

use alloc::string::String;

use core::ffi::{c_char, c_uint, c_void, CStr};
use core::ptr;
use core::time::Duration;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::Mutex;

use virtmcu_api::{
    FlatBufferStructExt, MmioReq, SyscMsg, VirtmcuHandshake, MMIO_REQ_READ, MMIO_REQ_WRITE,
    SYSC_MSG_IRQ_CLEAR, SYSC_MSG_IRQ_SET, SYSC_MSG_RESP, VIRTMCU_PROTO_MAGIC,
    VIRTMCU_PROTO_VERSION,
};
use virtmcu_qom::cosim::{CoSimBridge, CoSimContext, CoSimTransport};
use virtmcu_qom::irq::{qemu_set_irq, QemuIrq};
use virtmcu_qom::memory::{
    memory_region_init_io, MemoryRegion, MemoryRegionOps, DEVICE_LITTLE_ENDIAN,
};
use virtmcu_qom::qdev::SysBusDevice;
use virtmcu_qom::qdev::{sysbus_init_irq, sysbus_init_mmio, sysbus_mmio_map};
use virtmcu_qom::qom::{Object, ObjectClass, Property, TypeInfo};
use virtmcu_qom::sync::Bql;
use virtmcu_qom::timer::{qemu_clock_get_ns, QEMU_CLOCK_VIRTUAL};
use virtmcu_qom::{
    declare_device_type, define_prop_string, define_prop_uint32, define_prop_uint64, device_class,
    error_setg,
};

static MAPPED_IDS: Mutex<Option<HashMap<String, bool>>> = Mutex::new(None);

fn is_id_mapped(id: &str) -> bool {
    let mut lock = MAPPED_IDS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    *lock.get_or_insert_with(HashMap::new).get(id).unwrap_or(&false)
}

fn set_id_mapped(id: &str, mapped: bool) {
    let mut lock = MAPPED_IDS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    lock.get_or_insert_with(HashMap::new).insert(id.to_owned(), mapped);
}

#[repr(C)]
pub struct MmioSocketBridgeQEMU {
    pub parent_obj: SysBusDevice,
    pub mmio: MemoryRegion,

    // Properties mapped by QOM
    pub id: *mut c_char,
    pub socket_path: *mut c_char,
    pub region_size: u32,
    pub base_addr: u64,
    pub reconnect_ms: u32,

    pub irqs: [QemuIrq; 32],

    pub rust_state: *mut MmioSocketBridgeState,
    pub mapped: bool,
}

struct RawIrqArray(*mut QemuIrq);
// SAFETY: the IRQ array lives in MmioSocketBridgeQEMU which outlives SharedState.
// qemu_set_irq is only called while holding the BQL.
unsafe impl Send for RawIrqArray {}
unsafe impl Sync for RawIrqArray {}

struct MmioTransport {
    socket_path: String,
    reconnect_ms: u32,
    irqs: RawIrqArray,
    stream: Mutex<Option<UnixStream>>,
}

impl CoSimTransport for MmioTransport {
    type Request = MmioReq;
    type Response = SyscMsg;

    fn run_rx_loop(&self, ctx: &CoSimContext<Self::Response>) {
        loop {
            if !ctx.is_running() {
                break;
            }

            let stream_res = UnixStream::connect(&self.socket_path);
            let mut stream = match stream_res {
                Ok(s) => s,
                Err(e) => {
                    virtmcu_qom::sim_err!("connect failed to {}: {:?}", self.socket_path, e);
                    if self.reconnect_ms == 0 {
                        break;
                    }
                    std::thread::sleep /* SLEEP_EXCEPTION: Reconnect delay in background thread */(Duration::from_millis(u64::from(self.reconnect_ms)));
                    continue;
                }
            };

            // Handshake
            let hs_out = VirtmcuHandshake::new(VIRTMCU_PROTO_MAGIC, VIRTMCU_PROTO_VERSION);
            if stream.write_all(hs_out.pack()).is_err() {
                continue;
            }

            let mut hs_in_bytes = [0u8; virtmcu_api::VIRTMCU_HANDSHAKE_SIZE];
            if stream.read_exact(&mut hs_in_bytes).is_err() {
                continue;
            }
            let hs_in = VirtmcuHandshake::unpack_slice(&hs_in_bytes)
                .expect("Failed to unpack VirtmcuHandshake");
            if hs_in.magic() != VIRTMCU_PROTO_MAGIC || hs_in.version() != VIRTMCU_PROTO_VERSION {
                virtmcu_qom::sim_info!("handshake mismatch, retrying");
                continue;
            }

            let mut read_stream = match stream.try_clone() {
                Ok(rs) => rs,
                Err(_) => continue,
            };

            {
                let mut lock = self.stream.lock().unwrap();
                *lock = Some(stream);
                virtmcu_qom::sim_info!("connected to {}", self.socket_path);
            }
            ctx.notify_connected();

            // Blocking read loop
            loop {
                let mut msg_bytes = [0u8; virtmcu_api::SYSC_MSG_SIZE];
                if let Ok(()) = read_stream.read_exact(&mut msg_bytes) {
                    let msg = SyscMsg::unpack_slice(&msg_bytes).expect("Failed to unpack SyscMsg");
                    if msg.type_() == SYSC_MSG_RESP {
                        ctx.dispatch_response(msg);
                    } else if (msg.type_() == SYSC_MSG_IRQ_SET || msg.type_() == SYSC_MSG_IRQ_CLEAR)
                        && msg.irq_num() < 32
                    {
                        let bql = Bql::lock();
                        // SAFETY: irqs pointer is valid
                        unsafe {
                            qemu_set_irq(
                                *self.irqs.0.add(msg.irq_num() as usize),
                                i32::from(msg.type_() == SYSC_MSG_IRQ_SET),
                            );
                        }
                        drop(bql);
                    }
                } else {
                    {
                        let mut lock = self.stream.lock().unwrap();
                        *lock = None;
                    }
                    ctx.notify_disconnected();
                    virtmcu_qom::sim_info!("remote disconnected, closing socket");
                    break;
                }
            }
        }
    }

    fn send_request(&self, req: Self::Request) -> bool {
        let mut lock = self.stream.lock().unwrap();
        if let Some(s) = lock.as_mut() {
            s.write_all(req.pack()).is_ok()
        } else {
            false
        }
    }

    fn interrupt_rx(&self) {
        let mut lock = self.stream.lock().unwrap();
        if let Some(s) = lock.as_mut() {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
    }
}

pub struct MmioSocketBridgeState {
    bridge: CoSimBridge<MmioTransport>,
}

unsafe extern "C" fn bridge_read(opaque: *mut c_void, addr: u64, size: c_uint) -> u64 {
    let state = &*(opaque as *mut MmioSocketBridgeState);
    let req = MmioReq::new(
        MMIO_REQ_READ,
        size as u8,
        0,
        0,
        qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) as u64,
        addr,
        0,
    );

    // We attempt to wait if unconnected to preserve behavior
    state.bridge.wait_connected(5000);

    if let Some(resp) = state.bridge.send_and_wait(req, 5000) {
        resp.data()
    } else {
        0
    }
}

unsafe extern "C" fn bridge_write(opaque: *mut c_void, addr: u64, val: u64, size: c_uint) {
    let state = &*(opaque as *mut MmioSocketBridgeState);
    let req = MmioReq::new(
        MMIO_REQ_WRITE,
        size as u8,
        0,
        0,
        qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) as u64,
        addr,
        val,
    );

    state.bridge.wait_connected(5000);
    state.bridge.send_and_wait(req, 5000);
}

static BRIDGE_MMIO_OPS: MemoryRegionOps = MemoryRegionOps {
    read: Some(bridge_read),
    write: Some(bridge_write),
    read_with_attrs: ptr::null(),
    write_with_attrs: ptr::null(),
    endianness: DEVICE_LITTLE_ENDIAN,
    _padding1: [0; 4],
    valid: virtmcu_qom::memory::MemoryRegionValidRange {
        min_access_size: 0,
        max_access_size: 0,
        unaligned: false,
        _padding: [0; 7],
        accepts: ptr::null(),
    },
    impl_: virtmcu_qom::memory::MemoryRegionImplRange {
        min_access_size: 1,
        max_access_size: 8,
        unaligned: false,
        _padding: [0; 7],
    },
};

unsafe extern "C" fn bridge_realize(dev: *mut c_void, errp: *mut *mut c_void) {
    let qemu = &mut *(dev as *mut MmioSocketBridgeQEMU);
    let obj = dev as *mut Object;

    if qemu.socket_path.is_null() {
        error_setg!(errp, "socket-path must be set");
        return;
    }
    if qemu.region_size == 0 {
        error_setg!(errp, "region-size must be > 0");
        return;
    }

    for i in 0..32 {
        sysbus_init_irq(dev as *mut SysBusDevice, &raw mut qemu.irqs[i]);
    }

    let transport = MmioTransport {
        socket_path: CStr::from_ptr(qemu.socket_path).to_string_lossy().into_owned(),
        reconnect_ms: qemu.reconnect_ms,
        irqs: RawIrqArray(qemu.irqs.as_mut_ptr()),
        stream: Mutex::new(None),
    };

    let bridge = CoSimBridge::new(transport);
    let state = Box::new(MmioSocketBridgeState { bridge });
    qemu.rust_state = Box::into_raw(state);

    let id_str = if qemu.id.is_null() {
        None
    } else {
        Some(CStr::from_ptr(qemu.id).to_string_lossy().into_owned())
    };

    let already_mapped = if let Some(ref id) = id_str { is_id_mapped(id) } else { false };

    if !already_mapped {
        memory_region_init_io(
            &raw mut qemu.mmio,
            obj,
            &raw const BRIDGE_MMIO_OPS,
            qemu.rust_state as *mut c_void,
            c"mmio-socket-bridge".as_ptr(),
            u64::from(qemu.region_size),
        );

        sysbus_init_mmio(dev as *mut SysBusDevice, &raw mut qemu.mmio);

        if qemu.base_addr != u64::MAX {
            sysbus_mmio_map(dev as *mut SysBusDevice, 0, qemu.base_addr);
        }
        if let Some(ref id) = id_str {
            set_id_mapped(id, true);
        }
        qemu.mapped = true;
    }
}

unsafe extern "C" fn bridge_instance_init(_obj: *mut Object) {}
unsafe extern "C" fn bridge_instance_finalize(obj: *mut Object) {
    let qemu = &mut *(obj as *mut MmioSocketBridgeQEMU);
    if !qemu.rust_state.is_null() {
        let state = Box::from_raw(qemu.rust_state);
        drop(state); // CoSimBridge Drop handler cleans up thread and drains vCPUs!

        if qemu.mapped && !qemu.id.is_null() {
            let id = CStr::from_ptr(qemu.id).to_string_lossy().into_owned();
            set_id_mapped(&id, false);
        }

        qemu.rust_state = ptr::null_mut();
    }
}

unsafe extern "C" fn bridge_unrealize(_dev: *mut c_void) {}

static BRIDGE_PROPERTIES: [Property; 6] = [
    define_prop_string!(c"id".as_ptr(), MmioSocketBridgeQEMU, id),
    define_prop_string!(c"socket-path".as_ptr(), MmioSocketBridgeQEMU, socket_path),
    define_prop_uint32!(c"region-size".as_ptr(), MmioSocketBridgeQEMU, region_size, 0x1000),
    define_prop_uint64!(c"base-addr".as_ptr(), MmioSocketBridgeQEMU, base_addr, u64::MAX),
    define_prop_uint32!(c"reconnect-ms".as_ptr(), MmioSocketBridgeQEMU, reconnect_ms, 1000),
    // SAFETY: QEMU expects a zeroed Property as a sentinel.
    unsafe { core::mem::zeroed() },
];

unsafe extern "C" fn bridge_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let dc = device_class!(klass);
    (*dc).realize = Some(bridge_realize);
    (*dc).unrealize = Some(bridge_unrealize);
    (*dc).user_creatable = true;
    virtmcu_qom::qdev::device_class_set_props_n(dc, BRIDGE_PROPERTIES.as_ptr(), 5);
}

static BRIDGE_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"mmio-socket-bridge".as_ptr(),
    parent: c"sys-bus-device".as_ptr(),
    instance_size: core::mem::size_of::<MmioSocketBridgeQEMU>(),
    instance_align: 0,
    instance_init: Some(bridge_instance_init),
    instance_post_init: None,
    instance_finalize: Some(bridge_instance_finalize),
    abstract_: false,
    class_size: 0,
    class_init: Some(bridge_class_init),
    class_base_init: None,
    class_data: ptr::null(),
    interfaces: ptr::null(),
};

declare_device_type!(VIRTMCU_MMIO_SOCKET_BRIDGE_TYPE_INIT, BRIDGE_TYPE_INFO);
