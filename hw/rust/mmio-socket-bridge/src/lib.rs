//! # MMIO Socket Bridge
//!
//! Lock ordering: BQL -> SharedState Mutex -> (Condvar releases Mutex temporarily).
//! Background I/O thread never acquires BQL.
//! vCPU thread acquires BQL (held by QEMU), then locks SharedState Mutex, then
//! waits on Condvar (which releases Mutex). BQL is temporarily yielded during wait
//! via Bql::temporary_unlock().

use core::ffi::{c_char, c_uint, c_void};
use std::collections::HashMap;
use std::ffi::CStr;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::ptr;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use virtmcu_api::{
    MmioReq, SyscMsg, VirtmcuHandshake, MMIO_REQ_READ, MMIO_REQ_WRITE, SYSC_MSG_IRQ_CLEAR,
    SYSC_MSG_IRQ_SET, SYSC_MSG_RESP, VIRTMCU_PROTO_MAGIC, VIRTMCU_PROTO_VERSION,
};
use virtmcu_qom::irq::{qemu_irq, qemu_set_irq};
use virtmcu_qom::memory::{
    memory_region_init_io, MemoryRegion, MemoryRegionOps, DEVICE_LITTLE_ENDIAN,
};
use virtmcu_qom::qdev::SysBusDevice;
use virtmcu_qom::qdev::{sysbus_init_irq, sysbus_init_mmio, sysbus_mmio_map};
use virtmcu_qom::qom::{Object, ObjectClass, Property, TypeInfo};
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
    lock.get_or_insert_with(HashMap::new).insert(id.to_string(), mapped);
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

    pub irqs: [qemu_irq; 32],

    pub rust_state: *mut MmioSocketBridgeState,
    pub mapped: bool,
}

use virtmcu_qom::sync::Bql;

pub struct MmioSocketBridgeState {
    shared: Arc<SharedState>,
    bg_thread: Option<std::thread::JoinHandle<()>>,
}

pub struct SharedState {
    socket_path: String,
    reconnect_ms: u32,
    irqs: RawIrqArray, // Points to MmioSocketBridgeQEMU.irqs
    resp_cond: Condvar,
    connected_cond: Condvar,
    drain_cond: Condvar,
    state: Mutex<ConnectionState>,
}

// Safe: SharedState is only accessed through Arc and its internal synchronisation
// primitives (Mutex<ConnectionState>, Condvar). Raw pointers inside wrapper
// structs are documented below.
unsafe impl Send for SharedState {}
unsafe impl Sync for SharedState {}

struct RawIrqArray(*mut qemu_irq);
// Safe: the IRQ array lives in MmioSocketBridgeQEMU which outlives SharedState.
// qemu_set_irq is only called while holding the BQL.
unsafe impl Send for RawIrqArray {}
unsafe impl Sync for RawIrqArray {}

struct ConnectionState {
    stream: Option<UnixStream>,
    has_resp: bool,
    current_resp: Option<SyscMsg>,
    running: bool,
    active_vcpu_count: usize,
}

const BRIDGE_TIMEOUT_MS: u32 = 5000;
/// Maximum time to wait for all vCPU threads to exit during teardown.
/// Unbounded wait risks deadlocking QEMU if active_vcpu_count never reaches
/// zero due to a panic or logic bug in the vCPU path.
const DRAIN_TIMEOUT_SECS: u64 = 30;

impl Drop for SharedState {
    fn drop(&mut self) {
        // Rust Mutex and Condvar are automatically freed
    }
}

struct VcpuCountGuard<'a>(&'a SharedState);
impl Drop for VcpuCountGuard<'_> {
    fn drop(&mut self) {
        let mut lock = self.0.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        lock.active_vcpu_count = lock.active_vcpu_count.saturating_sub(1);
        if lock.active_vcpu_count == 0 {
            self.0.drain_cond.notify_all();
        }
    }
}

impl SharedState {
    fn run_background_thread(self: Arc<Self>) {
        loop {
            {
                let lock = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                if !lock.running {
                    break;
                }
            }

            let stream_res = UnixStream::connect(&self.socket_path);
            let mut stream = if let Ok(s) = stream_res {
                s
            } else {
                eprintln!(
                    "mmio-socket-bridge: connect failed to {}: {:?}",
                    self.socket_path,
                    stream_res.err()
                );
                if self.reconnect_ms == 0 {
                    break;
                }
                let d = Duration::from_millis(u64::from(self.reconnect_ms));
                let mut lock = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                let start = std::time::Instant::now();
                while lock.running && start.elapsed() < d {
                    let remaining =
                        d.checked_sub(start.elapsed()).unwrap_or(Duration::from_secs(0));
                    let (new_lock, _) = self
                        .connected_cond
                        .wait_timeout(lock, remaining)
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    lock = new_lock;
                }
                continue;
            };

            // Handshake: announce our magic/version and verify the server echoes it back.
            let hs_out =
                VirtmcuHandshake { magic: VIRTMCU_PROTO_MAGIC, version: VIRTMCU_PROTO_VERSION };
            let hs_bytes = hs_out.pack();
            if stream.write_all(&hs_bytes).is_err() {
                continue;
            }

            let mut hs_in_bytes = [0u8; 8];
            if stream.read_exact(&mut hs_in_bytes).is_err() {
                continue;
            }
            let hs_in = VirtmcuHandshake::unpack(&hs_in_bytes);
            if hs_in.magic != VIRTMCU_PROTO_MAGIC || hs_in.version != VIRTMCU_PROTO_VERSION {
                eprintln!("mmio-socket-bridge: handshake mismatch, retrying");
                continue;
            }

            let mut read_stream = match stream.try_clone() {
                Ok(rs) => rs,
                Err(_) => continue,
            };

            {
                let mut lock = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                lock.stream = Some(stream);
                eprintln!("mmio-socket-bridge: connected to {}", self.socket_path);
            }
            // Notify any vCPU thread blocked in send_req_and_wait_internal waiting for
            // a stream to become available. Without this, the waiter must spin out the
            // full BRIDGE_TIMEOUT_MS (5 s) before re-checking lock.stream.
            self.connected_cond.notify_all();

            // Blocking read loop (no timeout needed; shutdown() wakes it on teardown).
            loop {
                let mut msg_bytes = [0u8; 16]; // size of SyscMsg
                if let Ok(()) = read_stream.read_exact(&mut msg_bytes) {
                    let msg = SyscMsg::unpack(&msg_bytes);
                    if msg.type_ == SYSC_MSG_RESP {
                        let mut lock =
                            self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                        lock.current_resp = Some(msg);
                        lock.has_resp = true;
                        self.resp_cond.notify_all();
                    } else if (msg.type_ == SYSC_MSG_IRQ_SET || msg.type_ == SYSC_MSG_IRQ_CLEAR)
                        && msg.irq_num < 32
                    {
                        let bql = Bql::lock();
                        unsafe {
                            qemu_set_irq(
                                *self.irqs.0.add(msg.irq_num as usize),
                                i32::from(msg.type_ == SYSC_MSG_IRQ_SET),
                            );
                        }
                        drop(bql);
                    }
                } else {
                    // EOF or error — signal any waiting vCPU thread to return.
                    let mut lock =
                        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    lock.stream = None;
                    lock.has_resp = true;
                    lock.current_resp = None;
                    self.resp_cond.notify_all();
                    unsafe { virtmcu_qom::cpu::virtmcu_cpu_exit_all() };
                    eprintln!("mmio-socket-bridge: remote disconnected, closing socket");
                    break;
                }
            }
        }

        let mut lock = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        lock.running = false;
        self.connected_cond.notify_all();
        self.resp_cond.notify_all();
    }

    fn send_req_and_wait(&self, req: MmioReq) -> Option<SyscMsg> {
        {
            let mut lock = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if !lock.running {
                return None;
            }
            lock.active_vcpu_count += 1;
        }
        let _guard = VcpuCountGuard(self);

        self.send_req_and_wait_internal(req)
    }

    fn send_req_and_wait_internal(&self, req: MmioReq) -> Option<SyscMsg> {
        let mut lock = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            if !lock.running {
                return None;
            }
            if let Some(mut s) = lock.stream.take() {
                if s.write_all(&req.pack()).is_ok() {
                    lock.stream = Some(s);
                    lock.has_resp = false;
                    break;
                }
                // Write failed, stream is None
            }

            // Wait for connection
            let bql_unlock = Bql::temporary_unlock();
            let (new_lock, result) = self
                .connected_cond
                .wait_timeout(lock, Duration::from_millis(BRIDGE_TIMEOUT_MS as u64))
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            lock = new_lock;
            drop(bql_unlock);
            if result.timed_out() {
                eprintln!("mmio-socket-bridge: timeout waiting for connection");
                return None;
            }
        }

        // Wait for response
        while !lock.has_resp && lock.running {
            let bql_unlock = Bql::temporary_unlock();
            let (new_lock, result) = self
                .resp_cond
                .wait_timeout(lock, Duration::from_millis(BRIDGE_TIMEOUT_MS as u64))
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            lock = new_lock;
            drop(bql_unlock);
            if result.timed_out() {
                eprintln!("mmio-socket-bridge: timeout waiting for response");
                lock.stream = None;
                lock.has_resp = true;
                break;
            }
        }

        lock.current_resp.take()
    }
}

unsafe extern "C" fn bridge_read(opaque: *mut c_void, addr: u64, size: c_uint) -> u64 {
    let state = &*(opaque as *mut MmioSocketBridgeState);
    let req = MmioReq {
        type_: MMIO_REQ_READ,
        size: size as u8,
        reserved1: 0,
        reserved2: 0,
        vtime_ns: qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) as u64,
        addr,
        data: 0,
    };
    if let Some(resp) = state.shared.send_req_and_wait(req) {
        resp.data
    } else {
        0
    }
}

unsafe extern "C" fn bridge_write(opaque: *mut c_void, addr: u64, val: u64, size: c_uint) {
    let state = &*(opaque as *mut MmioSocketBridgeState);
    let req = MmioReq {
        type_: MMIO_REQ_WRITE,
        size: size as u8,
        reserved1: 0,
        reserved2: 0,
        vtime_ns: qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) as u64,
        addr,
        data: val,
    };
    state.shared.send_req_and_wait(req);
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

    let shared = Arc::new(SharedState {
        socket_path: CStr::from_ptr(qemu.socket_path).to_string_lossy().into_owned(),
        reconnect_ms: qemu.reconnect_ms,
        irqs: RawIrqArray(qemu.irqs.as_mut_ptr()),
        resp_cond: Condvar::new(),
        connected_cond: Condvar::new(),
        drain_cond: Condvar::new(),
        state: Mutex::new(ConnectionState {
            stream: None,
            has_resp: false,
            current_resp: None,
            running: true,
            active_vcpu_count: 0,
        }),
    });

    let shared_clone = Arc::clone(&shared);
    let bg_thread = std::thread::spawn(move || {
        shared_clone.run_background_thread();
    });

    let state =
        Box::new(MmioSocketBridgeState { shared: Arc::clone(&shared), bg_thread: Some(bg_thread) });
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
        let mut state = Box::from_raw(qemu.rust_state);
        // Stop background thread
        {
            let mut lock =
                state.shared.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            lock.running = false;
            if let Some(ref mut s) = lock.stream {
                let _ = s.shutdown(std::net::Shutdown::Both);
            }
        }
        // Wake it up if it's blocked on connect/wait
        state.shared.resp_cond.notify_all();
        state.shared.connected_cond.notify_all();

        // Wait for all vCPU threads to drain (bounded: avoids permanent deadlock
        // if active_vcpu_count never reaches zero due to a bug or panic).
        let mut lock = state.shared.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        while lock.active_vcpu_count > 0 {
            let bql_unlock = Bql::temporary_unlock();
            let (new_lock, timed_out) = state
                .shared
                .drain_cond
                .wait_timeout(lock, Duration::from_secs(DRAIN_TIMEOUT_SECS))
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            lock = new_lock;
            drop(bql_unlock);
            if timed_out.timed_out() {
                eprintln!(
                    "mmio-socket-bridge: drain timeout after {} s ({} vCPU threads still active); proceeding with teardown",
                    DRAIN_TIMEOUT_SECS, lock.active_vcpu_count
                );
                break;
            }
        }

        if let Some(handle) = state.bg_thread.take() {
            let bql_unlock = Bql::temporary_unlock();
            let _ = handle.join();
            drop(bql_unlock);
        }

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
    unsafe { std::mem::zeroed() },
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
    instance_size: std::mem::size_of::<MmioSocketBridgeQEMU>(),
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

declare_device_type!(virtmcu_mmio_socket_bridge_init, BRIDGE_TYPE_INFO);
