use core::ffi::{c_char, c_uint, c_void};
use std::ffi::CStr;
use std::ptr;
use virtmcu_qom::irq::{qemu_irq, qemu_set_irq};
use virtmcu_qom::memory::{
    memory_region_init_io, MemoryRegion, MemoryRegionOps, DEVICE_NATIVE_ENDIAN,
};
use virtmcu_qom::qdev::SysBusDevice;
use virtmcu_qom::qdev::{sysbus_init_irq, sysbus_init_mmio, sysbus_mmio_map};
use virtmcu_qom::qom::{Object, ObjectClass, Property, TypeInfo};
use virtmcu_qom::sync::{Bql, QemuCond, QemuMutex};
use virtmcu_qom::{
    declare_device_type, define_prop_string, define_prop_uint32, define_prop_uint64, device_class,
    error_setg,
};

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

// --- Remote Port Protocol Definitions ---

pub const RP_VERSION_MAJOR: u16 = 4;
pub const RP_VERSION_MINOR: u16 = 3;

#[repr(u32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum RpCmd {
    Nop = 0,
    Hello = 1,
    Cfg = 2,
    Read = 3,
    Write = 4,
    Interrupt = 5,
    Sync = 6,
    AtsReq = 7,
    AtsInv = 8,
}

pub const RP_PKT_FLAGS_RESPONSE: u32 = 1 << 1;
pub const RP_PKT_FLAGS_POSTED: u32 = 1 << 2;

#[repr(C, packed)]
#[derive(Debug, Copy, Clone)]
pub struct RpPktHdr {
    pub cmd: u32,
    pub len: u32,
    pub id: u32,
    pub flags: u32,
    pub dev: u32,
}

impl RpPktHdr {
    pub fn to_be(&self) -> Self {
        Self {
            cmd: self.cmd.to_be(),
            len: self.len.to_be(),
            id: self.id.to_be(),
            flags: self.flags.to_be(),
            dev: self.dev.to_be(),
        }
    }

    pub fn from_be(&self) -> Self {
        Self {
            cmd: u32::from_be(self.cmd),
            len: u32::from_be(self.len),
            id: u32::from_be(self.id),
            flags: u32::from_be(self.flags),
            dev: u32::from_be(self.dev),
        }
    }
}

#[repr(C, packed)]
#[derive(Debug, Copy, Clone)]
pub struct RpVersion {
    pub major: u16,
    pub minor: u16,
}

#[repr(C, packed)]
#[derive(Debug, Copy, Clone)]
pub struct RpCapabilities {
    pub offset: u32,
    pub len: u16,
    pub reserved0: u16,
}

#[repr(C, packed)]
#[derive(Debug, Copy, Clone)]
pub struct RpPktHello {
    pub hdr: RpPktHdr,
    pub version: RpVersion,
    pub caps: RpCapabilities,
}

#[repr(C, packed)]
#[derive(Debug, Copy, Clone)]
pub struct RpPktBusaccess {
    pub hdr: RpPktHdr,
    pub timestamp: u64,
    pub attributes: u64,
    pub addr: u64,
    pub len: u32,
    pub width: u32,
    pub stream_width: u32,
    pub master_id: u16,
}

impl RpPktBusaccess {
    pub fn to_be(&self) -> Self {
        Self {
            hdr: self.hdr.to_be(),
            timestamp: self.timestamp.to_be(),
            attributes: self.attributes.to_be(),
            addr: self.addr.to_be(),
            len: self.len.to_be(),
            width: self.width.to_be(),
            stream_width: self.stream_width.to_be(),
            master_id: self.master_id.to_be(),
        }
    }

    pub fn from_be(&self) -> Self {
        Self {
            hdr: self.hdr.from_be(),
            timestamp: u64::from_be(self.timestamp),
            attributes: u64::from_be(self.attributes),
            addr: u64::from_be(self.addr),
            len: u32::from_be(self.len),
            width: u32::from_be(self.width),
            stream_width: u32::from_be(self.stream_width),
            master_id: u16::from_be(self.master_id),
        }
    }
}

#[repr(C, packed)]
#[derive(Debug, Copy, Clone)]
pub struct RpPktInterrupt {
    pub hdr: RpPktHdr,
    pub timestamp: u64,
    pub vector: u64,
    pub line: u32,
    pub val: u8,
}

impl RpPktInterrupt {
    pub fn from_be(&self) -> Self {
        Self {
            hdr: self.hdr.from_be(),
            timestamp: u64::from_be(self.timestamp),
            vector: u64::from_be(self.vector),
            line: u32::from_be(self.line),
            val: self.val,
        }
    }
}

// --- QOM Device Implementation ---

#[repr(C)]
pub struct RemotePortBridgeQEMU {
    pub parent_obj: SysBusDevice,
    pub mmio: MemoryRegion,

    pub socket_path: *mut c_char,
    pub region_size: u32,
    pub base_addr: u64,
    pub reconnect_ms: u32,

    pub irqs: [qemu_irq; 32],

    pub rust_state: *mut RemotePortBridgeState,
}

pub struct RemotePortBridgeState {
    shared: Arc<SharedState>,
}

pub struct SharedState {
    socket_path: String,
    reconnect_ms: u32,
    irqs: RawIrqArray,
    conn: RawQemuMutex,
    resp_cond: RawQemuCond,
    state: Mutex<ConnectionState>,
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
    current_resp: Option<RpPktBusaccess>,
    current_data: [u8; 8],
    next_id: u32,
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
        let mut rx_buf = Vec::with_capacity(4096);
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
                if self.reconnect_ms > 0 {
                    std::thread::sleep(Duration::from_millis(self.reconnect_ms as u64));
                    continue;
                } else {
                    eprintln!(
                        "remote-port-bridge: failed to connect to {}, exiting thread",
                        self.socket_path
                    );
                    break;
                }
            };

            // Handshake
            let hello = RpPktHello {
                hdr: RpPktHdr {
                    cmd: RpCmd::Hello as u32,
                    len: (std::mem::size_of::<RpVersion>() + std::mem::size_of::<RpCapabilities>())
                        as u32,
                    id: 0,
                    flags: 0,
                    dev: 0,
                }
                .to_be(),
                version: RpVersion {
                    major: RP_VERSION_MAJOR.to_be(),
                    minor: RP_VERSION_MINOR.to_be(),
                },
                caps: RpCapabilities {
                    offset: (std::mem::size_of::<RpPktHello>() as u32).to_be(),
                    len: 0,
                    reserved0: 0,
                },
            };

            let hello_bytes = unsafe {
                std::slice::from_raw_parts(
                    &hello as *const _ as *const u8,
                    std::mem::size_of::<RpPktHello>(),
                )
            };

            if stream.write_all(hello_bytes).is_err() {
                continue;
            }

            let mut read_stream = match stream.try_clone() {
                Ok(rs) => rs,
                Err(_) => continue,
            };

            {
                let mut lock = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                lock.stream = Some(stream);
                eprintln!("remote-port-bridge: connected to {}", self.socket_path);
            }

            // Read loop
            let mut temp_buf = [0u8; 1024];
            loop {
                match read_stream.read(&mut temp_buf) {
                    Ok(0) => break, // EOF
                    Ok(n) => {
                        rx_buf.extend_from_slice(&temp_buf[..n]);
                        while rx_buf.len() >= std::mem::size_of::<RpPktHdr>() {
                            let hdr_be = unsafe { *(rx_buf.as_ptr() as *const RpPktHdr) };
                            let hdr = hdr_be.from_be();
                            let pkt_len = std::mem::size_of::<RpPktHdr>() + hdr.len as usize;

                            if rx_buf.len() < pkt_len {
                                break;
                            }

                            self.handle_packet(&rx_buf[..pkt_len], &hdr);
                            rx_buf.drain(..pkt_len);
                        }
                    }
                    Err(_) => break,
                }
            }

            {
                let mut lock = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                lock.stream = None;
                lock.has_resp = true;
                unsafe { (*self.resp_cond.0).broadcast() };
                eprintln!("remote-port-bridge: remote disconnected");
            }

            if self.reconnect_ms == 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(self.reconnect_ms as u64));
        }
    }

    fn handle_packet(&self, data: &[u8], hdr: &RpPktHdr) {
        if hdr.cmd == RpCmd::Interrupt as u32 {
            if data.len() >= std::mem::size_of::<RpPktInterrupt>() {
                let pkt_be = unsafe { *(data.as_ptr() as *const RpPktInterrupt) };
                let pkt = pkt_be.from_be();
                if pkt.line < 32 {
                    let bql = Bql::lock();
                    unsafe {
                        qemu_set_irq(
                            *self.irqs.0.add(pkt.line as usize),
                            if pkt.val != 0 { 1 } else { 0 },
                        );
                    }
                    drop(bql);
                }
            }
        } else {
            let mut lock = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if hdr.cmd == RpCmd::Read as u32 || hdr.cmd == RpCmd::Write as u32 {
                if data.len() >= std::mem::size_of::<RpPktBusaccess>() {
                    let pkt_be = unsafe { *(data.as_ptr() as *const RpPktBusaccess) };
                    lock.current_resp = Some(pkt_be.from_be());

                    let bus_hdr_len =
                        std::mem::size_of::<RpPktBusaccess>() - std::mem::size_of::<RpPktHdr>();
                    let payload_len = hdr.len as usize - bus_hdr_len;
                    if payload_len > 0 && payload_len <= 8 {
                        lock.current_data[..payload_len].copy_from_slice(
                            &data[std::mem::size_of::<RpPktBusaccess>()
                                ..std::mem::size_of::<RpPktBusaccess>() + payload_len],
                        );
                    }
                }
            } else if hdr.cmd == RpCmd::Hello as u32 {
                // Handshake response received
            }
            lock.has_resp = true;
            unsafe { (*self.resp_cond.0).broadcast() };
        }
    }

    fn send_req_and_wait(
        &self,
        cmd: RpCmd,
        addr: u64,
        size: u32,
        data_to_write: Option<&[u8]>,
    ) -> Option<u64> {
        let mut id;

        // Wait for connection
        loop {
            {
                let mut lock = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                if !lock.running {
                    return None;
                }
                if let Some(mut stream) = lock.stream.take() {
                    id = lock.next_id;
                    lock.next_id += 1;
                    lock.has_resp = false;
                    lock.current_resp = None;

                    let bus_hdr_len = (std::mem::size_of::<RpPktBusaccess>()
                        - std::mem::size_of::<RpPktHdr>())
                        as u32;
                    let payload_len = data_to_write.map(|d| d.len()).unwrap_or(0) as u32;

                    let pkt = RpPktBusaccess {
                        hdr: RpPktHdr {
                            cmd: cmd as u32,
                            len: bus_hdr_len + payload_len,
                            id,
                            flags: 0,
                            dev: 0,
                        }
                        .to_be(),
                        timestamp: 0,
                        attributes: 0,
                        addr: addr.to_be(),
                        len: size.to_be(),
                        width: size.to_be(),
                        stream_width: size.to_be(),
                        master_id: 0,
                    };

                    let pkt_bytes = unsafe {
                        std::slice::from_raw_parts(
                            &pkt as *const _ as *const u8,
                            std::mem::size_of::<RpPktBusaccess>(),
                        )
                    };

                    let mut send_success = false;
                    if stream.write_all(pkt_bytes).is_ok() {
                        let mut failed = false;
                        if let Some(d) = data_to_write {
                            if stream.write_all(d).is_err() {
                                failed = true;
                            }
                        }
                        if !failed {
                            send_success = true;
                        }
                    }

                    if send_success {
                        lock.stream = Some(stream);
                        break; // Successfully sent
                    }

                    // Write failed, already taken out of lock
                }
            }
            // Sleep and retry
            let _bql_unlock = if unsafe { virtmcu_qom::sync::virtmcu_bql_locked() } {
                Some(Bql::temporary_unlock())
            } else {
                None
            };
            std::thread::sleep(Duration::from_millis(10));
        }

        unsafe {
            virtmcu_qom::sync::virtmcu_mutex_lock(self.conn.0);
            let _bql_unlock = if virtmcu_qom::sync::virtmcu_bql_locked() {
                Some(Bql::temporary_unlock())
            } else {
                None
            };

            while !self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).has_resp {
                if !(*self.resp_cond.0).wait_timeout(&mut *self.conn.0, BRIDGE_TIMEOUT_MS) {
                    eprintln!("remote-port-bridge: timeout");
                    let mut lock =
                        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    lock.stream = None;
                    lock.has_resp = true;
                    break;
                }
            }
            virtmcu_qom::sync::virtmcu_mutex_unlock(self.conn.0);
        }

        let lock = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if cmd == RpCmd::Read {
            let mut val = 0u64;
            let n = size as usize;
            if n <= 8 {
                unsafe {
                    ptr::copy_nonoverlapping(
                        lock.current_data.as_ptr(),
                        &mut val as *mut u64 as *mut u8,
                        n,
                    );
                }
            }
            Some(val)
        } else {
            Some(0)
        }
    }
}

unsafe extern "C" fn bridge_read(opaque: *mut c_void, addr: u64, size: c_uint) -> u64 {
    let state = &*(opaque as *mut RemotePortBridgeState);
    state.shared.send_req_and_wait(RpCmd::Read, addr, size, None).unwrap_or(0)
}

unsafe extern "C" fn bridge_write(opaque: *mut c_void, addr: u64, val: u64, size: c_uint) {
    let state = &*(opaque as *mut RemotePortBridgeState);
    let data = val.to_ne_bytes();
    state.shared.send_req_and_wait(RpCmd::Write, addr, size, Some(&data[..size as usize]));
}

static BRIDGE_MMIO_OPS: MemoryRegionOps = MemoryRegionOps {
    read: Some(bridge_read),
    write: Some(bridge_write),
    read_with_attrs: ptr::null(),
    write_with_attrs: ptr::null(),
    endianness: DEVICE_NATIVE_ENDIAN,
    _padding1: [0; 4],
    valid: virtmcu_qom::memory::MemoryRegionValidRange {
        min_access_size: 1,
        max_access_size: 8,
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
    let qemu = &mut *(dev as *mut RemotePortBridgeQEMU);
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
            current_data: [0u8; 8],
            next_id: 0,
            running: true,
        }),
    });

    let state = Box::new(RemotePortBridgeState { shared: Arc::clone(&shared) });
    qemu.rust_state = Box::into_raw(state);

    std::thread::spawn(move || {
        shared.run_background_thread();
    });

    memory_region_init_io(
        &raw mut qemu.mmio,
        dev as *mut Object,
        &raw const BRIDGE_MMIO_OPS,
        qemu.rust_state as *mut c_void,
        c"remote-port-bridge".as_ptr(),
        u64::from(qemu.region_size),
    );

    sysbus_init_mmio(dev as *mut SysBusDevice, &raw mut qemu.mmio);

    if qemu.base_addr != 0 {
        sysbus_mmio_map(dev as *mut SysBusDevice, 0, qemu.base_addr);
    }
}

unsafe extern "C" fn bridge_instance_init(_obj: *mut Object) {}

unsafe extern "C" fn bridge_instance_finalize(obj: *mut Object) {
    let qemu = &mut *(obj as *mut RemotePortBridgeQEMU);
    if !qemu.rust_state.is_null() {
        let state = Box::from_raw(qemu.rust_state);
        {
            let mut lock = state.shared.state.lock().unwrap();
            lock.running = false;
            if let Some(ref mut s) = lock.stream {
                let _ = s.shutdown(std::net::Shutdown::Both);
            }
        }
        unsafe {
            (*state.shared.resp_cond.0).broadcast();
        }
        qemu.rust_state = ptr::null_mut();
    }
}

unsafe extern "C" fn bridge_unrealize(_dev: *mut c_void) {}

static mut BRIDGE_PROPERTIES: [Property; 5] = [
    define_prop_string!(c"socket-path".as_ptr(), RemotePortBridgeQEMU, socket_path),
    define_prop_uint32!(c"region-size".as_ptr(), RemotePortBridgeQEMU, region_size, 0x1000),
    define_prop_uint64!(c"base-addr".as_ptr(), RemotePortBridgeQEMU, base_addr, 0),
    define_prop_uint32!(c"reconnect-ms".as_ptr(), RemotePortBridgeQEMU, reconnect_ms, 1000),
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
    name: c"remote-port-bridge".as_ptr(),
    parent: c"sys-bus-device".as_ptr(),
    instance_size: std::mem::size_of::<RemotePortBridgeQEMU>(),
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

declare_device_type!(remote_port_bridge_type_init, BRIDGE_TYPE_INFO);
