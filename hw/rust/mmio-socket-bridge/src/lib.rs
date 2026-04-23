use core::ffi::{c_char, c_uint, c_void};
use std::ffi::CStr;
use std::ptr;
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

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[repr(C)]
pub struct MmioSocketBridgeQEMU {
    pub parent_obj: SysBusDevice,
    pub mmio: MemoryRegion,

    // Properties mapped by QOM
    pub socket_path: *mut c_char,
    pub region_size: u32,
    pub base_addr: u64,
    pub reconnect_ms: u32,

    pub irqs: [qemu_irq; 32],

    pub rust_state: *mut MmioSocketBridgeState,
}

use virtmcu_qom::sync::{Bql, QemuCond, QemuMutex};

pub struct MmioSocketBridgeState {
    shared: Arc<SharedState>,
    bg_thread: Option<std::thread::JoinHandle<()>>,
}

use std::sync::atomic::{AtomicUsize, Ordering};

pub struct SharedState {
    socket_path: String,
    reconnect_ms: u32,
    irqs: RawIrqArray, // Points to MmioSocketBridgeQEMU.irqs
    conn: RawQemuMutex,
    resp_cond: RawQemuCond,
    state: Mutex<ConnectionState>,
    active_vcpu_count: AtomicUsize,
}

unsafe impl Send for SharedState {}
unsafe impl Sync for SharedState {}

struct RawIrqArray(*mut qemu_irq);
unsafe impl Send for RawIrqArray {}
unsafe impl Sync for RawIrqArray {}

struct RawQemuMutex(*mut QemuMutex);
unsafe impl Send for RawQemuMutex {}
unsafe impl Sync for RawQemuMutex {}

struct RawQemuCond(*mut QemuCond);
unsafe impl Send for RawQemuCond {}
unsafe impl Sync for RawQemuCond {}

struct ConnectionState {
    stream: Option<UnixStream>,
    has_resp: bool,
    current_resp: Option<SyscMsg>,
    running: bool,
}

const BRIDGE_TIMEOUT_MS: u32 = 5000;

impl Drop for SharedState {
    fn drop(&mut self) {
        unsafe {
            virtmcu_qom::sync::virtmcu_mutex_free(self.conn.0);
            virtmcu_qom::sync::virtmcu_cond_free(self.resp_cond.0);
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
                std::thread::sleep(Duration::from_millis(self.reconnect_ms as u64));
                continue;
            };

            // Handshake
            let hs_out =
                VirtmcuHandshake { magic: VIRTMCU_PROTO_MAGIC, version: VIRTMCU_PROTO_VERSION };
            let hs_bytes: [u8; 8] = unsafe { core::mem::transmute(hs_out) };
            if stream.write_all(&hs_bytes).is_err() {
                continue;
            }

            let mut hs_in_bytes = [0u8; 8];
            if stream.read_exact(&mut hs_in_bytes).is_err() {
                continue;
            }
            let hs_in: VirtmcuHandshake = unsafe { core::mem::transmute(hs_in_bytes) };
            if hs_in.magic != VIRTMCU_PROTO_MAGIC || hs_in.version != VIRTMCU_PROTO_VERSION {
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

            // Blocking read loop (no timeout needed, shutdown will wake it)
            loop {
                let mut msg_bytes = [0u8; 16]; // size of SyscMsg
                if let Ok(()) = read_stream.read_exact(&mut msg_bytes) {
                    let msg: SyscMsg = unsafe { core::mem::transmute(msg_bytes) };
                    if msg.type_ == SYSC_MSG_RESP {
                        let mut lock =
                            self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                        lock.current_resp = Some(msg);
                        lock.has_resp = true;
                        unsafe { (*self.resp_cond.0).broadcast() };
                    } else if (msg.type_ == SYSC_MSG_IRQ_SET || msg.type_ == SYSC_MSG_IRQ_CLEAR)
                        && msg.irq_num < 32
                    {
                        // Important: We MUST acquire the BQL before calling qemu_set_irq
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
                    // EOF or error or shutdown
                    let mut lock =
                        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    lock.stream = None;
                    lock.has_resp = true; // Wake up blocked vCPU to return 0
                    lock.current_resp = None;
                    unsafe { (*self.resp_cond.0).broadcast() };
                    unsafe { virtmcu_qom::cpu::virtmcu_cpu_exit_all() };
                    eprintln!("mmio-socket-bridge: remote disconnected, closing socket");
                    break;
                }
            }
        }
    }
}

impl SharedState {
    fn send_req_and_wait(&self, req: MmioReq) -> Option<SyscMsg> {
        self.active_vcpu_count.fetch_add(1, Ordering::SeqCst);
        let res = self.send_req_and_wait_internal(req);
        self.active_vcpu_count.fetch_sub(1, Ordering::SeqCst);
        res
    }

    fn send_req_and_wait_internal(&self, req: MmioReq) -> Option<SyscMsg> {
        let req_bytes: [u8; 32] = unsafe { core::mem::transmute(req) };

        // Wait for connection if not ready
        loop {
            {
                let mut state_lock =
                    self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);

                if !state_lock.running {
                    return None;
                }

                let mut success = false;
                if let Some(stream) = state_lock.stream.as_mut() {
                    if stream.write_all(&req_bytes).is_err() {
                        eprintln!("mmio-socket-bridge: write_all failed!");
                        return None;
                    }
                    success = true;
                }
                if success {
                    state_lock.has_resp = false;
                    eprintln!("mmio-socket-bridge: sent req!");
                    break; // Successfully sent
                }
            }

            // Not connected yet. Drop lock, unlock BQL, and sleep briefly.
            let _bql_unlock = Bql::temporary_unlock();
            std::thread::sleep(Duration::from_millis(10));
        }

        // Wait for response using QEMU native primitives
        unsafe {
            virtmcu_qom::sync::virtmcu_mutex_lock(self.conn.0);

            // Temporarily unlock BQL if we hold it, so other threads (like QMP) can run.
            // We use our shim virtmcu_bql_locked() because the native bql_locked()
            // might return False in a DSO due to TLS issues.
            let _bql_unlock = Bql::temporary_unlock();

            while !self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).has_resp {
                if !(*self.resp_cond.0).wait_timeout(&mut *self.conn.0, BRIDGE_TIMEOUT_MS) {
                    // Timeout
                    eprintln!(
                        "mmio-socket-bridge: timeout on socket after {BRIDGE_TIMEOUT_MS} ms, disconnecting"
                    );
                    let mut state_lock =
                        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    state_lock.stream = None;
                    state_lock.has_resp = true;
                    // Yield CPU so QMP and other threads can run
                    virtmcu_qom::cpu::virtmcu_cpu_exit_all();
                    break;
                }
            }

            virtmcu_qom::sync::virtmcu_mutex_unlock(self.conn.0);
        }

        let mut state_lock = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let resp = state_lock.current_resp;
        state_lock.current_resp = None;
        resp
    }
}

unsafe extern "C" fn bridge_read(opaque: *mut c_void, addr: u64, size: c_uint) -> u64 {
    let state = &*(opaque as *mut MmioSocketBridgeState);
    let req = MmioReq {
        type_: MMIO_REQ_READ,
        size: size as u8,
        reserved1: 0,
        reserved2: 0,
        vtime_ns: unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64,
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
        vtime_ns: unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64,
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
    eprintln!("mmio-socket-bridge: bridge_realize CALLED!");
    let qemu = &mut *(dev as *mut MmioSocketBridgeQEMU);
    if qemu.socket_path.is_null() {
        error_setg!(errp, "socket-path must be set");
        return;
    }
    if qemu.region_size == 0 {
        error_setg!(errp, "region-size must be > 0");
        return;
    }

    let socket_path = CStr::from_ptr(qemu.socket_path).to_string_lossy().into_owned();

    for i in 0..32 {
        sysbus_init_irq(dev as *mut SysBusDevice, &raw mut qemu.irqs[i]);
    }

    let conn_ptr = virtmcu_qom::sync::virtmcu_mutex_new();
    let resp_cond_ptr = virtmcu_qom::sync::virtmcu_cond_new();

    let shared = Arc::new(SharedState {
        socket_path,
        reconnect_ms: qemu.reconnect_ms,
        irqs: RawIrqArray(qemu.irqs.as_mut_ptr()),
        conn: RawQemuMutex(conn_ptr),
        resp_cond: RawQemuCond(resp_cond_ptr),
        state: Mutex::new(ConnectionState {
            stream: None,
            has_resp: false,
            current_resp: None,
            running: true,
        }),
        active_vcpu_count: AtomicUsize::new(0),
    });

    let shared_clone = Arc::clone(&shared);
    let bg_thread = std::thread::spawn(move || {
        shared_clone.run_background_thread();
    });

    let state =
        Box::new(MmioSocketBridgeState { shared: Arc::clone(&shared), bg_thread: Some(bg_thread) });
    qemu.rust_state = Box::into_raw(state);

    memory_region_init_io(
        &raw mut qemu.mmio,
        dev as *mut Object,
        &raw const BRIDGE_MMIO_OPS,
        qemu.rust_state as *mut c_void,
        c"mmio-socket-bridge".as_ptr(),
        u64::from(qemu.region_size),
    );

    sysbus_init_mmio(dev as *mut SysBusDevice, &raw mut qemu.mmio);

    if qemu.base_addr != u64::MAX {
        unsafe {
            sysbus_mmio_map(dev as *mut SysBusDevice, 0, qemu.base_addr);
        }
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
        unsafe {
            (*state.shared.resp_cond.0).broadcast();
        }

        if let Some(handle) = state.bg_thread.take() {
            let _ = handle.join();
        }

        // Wait for any active vCPU threads to exit send_req_and_wait.
        // This ensures they are no longer using the mutex/condvar before we free them.
        let mut attempts = 0;
        while state.shared.active_vcpu_count.load(Ordering::SeqCst) > 0 && attempts < 1000 {
            std::thread::yield_now();
            attempts += 1;
        }

        qemu.rust_state = ptr::null_mut();
    }
}

unsafe extern "C" fn bridge_unrealize(_dev: *mut c_void) {}

static mut BRIDGE_PROPERTIES: [Property; 5] = [
    define_prop_string!(c"socket-path".as_ptr(), MmioSocketBridgeQEMU, socket_path),
    define_prop_uint32!(c"region-size".as_ptr(), MmioSocketBridgeQEMU, region_size, 0),
    define_prop_uint64!(c"base-addr".as_ptr(), MmioSocketBridgeQEMU, base_addr, u64::MAX),
    define_prop_uint32!(c"reconnect-ms".as_ptr(), MmioSocketBridgeQEMU, reconnect_ms, 0),
    // Null terminator
    unsafe { std::mem::zeroed() },
];

#[allow(static_mut_refs)]
unsafe extern "C" fn bridge_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let dc = device_class!(klass);
    (*dc).realize = Some(bridge_realize);
    (*dc).unrealize = Some(bridge_unrealize);
    (*dc).user_creatable = true;
    virtmcu_qom::qdev::device_class_set_props_n(dc, BRIDGE_PROPERTIES.as_ptr(), 4);
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

declare_device_type!(mmio_socket_bridge_type_init, BRIDGE_TYPE_INFO);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mmio_socket_bridge_qemu_layout() {
        assert_eq!(
            core::mem::offset_of!(MmioSocketBridgeQEMU, parent_obj),
            0,
            "SysBusDevice must be the first field"
        );
    }
}
