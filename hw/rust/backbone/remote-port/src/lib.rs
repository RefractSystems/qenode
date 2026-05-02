//! # Remote Port Bridge
//!
//! Lock ordering: BQL -> SharedState Mutex -> (Condvar releases Mutex temporarily).
//! Background I/O thread never acquires BQL.
//! vCPU thread acquires BQL (held by QEMU), then locks SharedState Mutex, then
//! waits on Condvar (which releases Mutex). BQL is temporarily yielded during wait
//! via Bql::temporary_unlock().

use core::ffi::CStr;
use core::ffi::{c_char, c_uint, c_void};
use core::ptr;
use virtmcu_qom::irq::{qemu_set_irq, QemuIrq};
use virtmcu_qom::memory::{
    memory_region_init_io, MemoryRegion, MemoryRegionOps, DEVICE_NATIVE_ENDIAN,
};
use virtmcu_qom::qdev::SysBusDevice;
use virtmcu_qom::qdev::{sysbus_init_irq, sysbus_init_mmio, sysbus_mmio_map};
use virtmcu_qom::qom::{Object, ObjectClass, Property, TypeInfo};
use virtmcu_qom::sync::Bql;

use virtmcu_qom::cosim::{CoSimBridge, CoSimContext, CoSimTransport};
use virtmcu_qom::{
    declare_device_type, define_prop_string, define_prop_uint32, define_prop_uint64, device_class,
    error_setg,
};

use core::time::Duration;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::Mutex;

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
    /// Serialize to big-endian wire bytes without raw memory cast.
    pub fn pack_be(&self) -> [u8; 20] {
        let mut b = [0u8; 20];
        b[0..4].copy_from_slice(&self.cmd.to_be_bytes());
        b[4..8].copy_from_slice(&self.len.to_be_bytes());
        b[8..12].copy_from_slice(&self.id.to_be_bytes());
        b[12..16].copy_from_slice(&self.flags.to_be_bytes());
        b[16..20].copy_from_slice(&self.dev.to_be_bytes());
        b
    }

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

impl RpVersion {
    pub fn pack_be(&self) -> [u8; 4] {
        let mut b = [0u8; 4];
        b[0..2].copy_from_slice(&self.major.to_be_bytes());
        b[2..4].copy_from_slice(&self.minor.to_be_bytes());
        b
    }
}

#[repr(C, packed)]
#[derive(Debug, Copy, Clone)]
pub struct RpCapabilities {
    pub offset: u32,
    pub len: u16,
    pub reserved0: u16,
}

impl RpCapabilities {
    pub fn pack_be(&self) -> [u8; 8] {
        let mut b = [0u8; 8];
        b[0..4].copy_from_slice(&self.offset.to_be_bytes());
        b[4..6].copy_from_slice(&self.len.to_be_bytes());
        b[6..8].copy_from_slice(&self.reserved0.to_be_bytes());
        b
    }
}

#[repr(C, packed)]
#[derive(Debug, Copy, Clone)]
pub struct RpPktHello {
    pub hdr: RpPktHdr,
    pub version: RpVersion,
    pub caps: RpCapabilities,
}

impl RpPktHello {
    pub fn pack_be(&self) -> [u8; 32] {
        let mut b = [0u8; 32];
        b[0..20].copy_from_slice(&self.hdr.pack_be());
        b[20..24].copy_from_slice(&self.version.pack_be());
        b[24..32].copy_from_slice(&self.caps.pack_be());
        b
    }
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
    pub fn pack_be(&self) -> [u8; 58] {
        let mut b = [0u8; 58];
        b[0..20].copy_from_slice(&self.hdr.pack_be());
        b[20..28].copy_from_slice(&self.timestamp.to_be_bytes());
        b[28..36].copy_from_slice(&self.attributes.to_be_bytes());
        b[36..44].copy_from_slice(&self.addr.to_be_bytes());
        b[44..48].copy_from_slice(&self.len.to_be_bytes());
        b[48..52].copy_from_slice(&self.width.to_be_bytes());
        b[52..56].copy_from_slice(&self.stream_width.to_be_bytes());
        b[56..58].copy_from_slice(&self.master_id.to_be_bytes());
        b
    }
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
    pub fn pack_be(&self) -> [u8; 41] {
        let mut b = [0u8; 41];
        b[0..20].copy_from_slice(&self.hdr.pack_be());
        b[20..28].copy_from_slice(&self.timestamp.to_be_bytes());
        b[28..36].copy_from_slice(&self.vector.to_be_bytes());
        b[36..40].copy_from_slice(&self.line.to_be_bytes());
        b[40] = self.val;
        b
    }
    pub fn to_be(&self) -> Self {
        Self {
            hdr: self.hdr.to_be(),
            timestamp: self.timestamp.to_be(),
            vector: self.vector.to_be(),
            line: self.line.to_be(),
            val: self.val,
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use core::ptr;

    #[test]
    fn test_unaligned_hdr_read() {
        #[repr(C, align(8))]
        struct AlignedBuf([u8; 32]);
        let mut buf_wrapper = AlignedBuf([0u8; 32]);
        let buf = &mut buf_wrapper.0;
        let hdr = RpPktHdr {
            cmd: 0x11223344,
            len: 0x55667788,
            id: 0x99AABBCC,
            flags: 0xDDEEFF00,
            dev: 0x12345678,
        }
        .to_be();

        // Write at offset 1 to force misalignment
        let hdr_ptr = &hdr as *const RpPktHdr as *const u8;
        unsafe {
            ptr::copy_nonoverlapping(
                hdr_ptr,
                buf.as_mut_ptr().add(1),
                core::mem::size_of::<RpPktHdr>(),
            );
        }

        let misaligned_ptr = unsafe { buf.as_ptr().add(1) } as *const RpPktHdr;
        assert!(!(misaligned_ptr as usize).is_multiple_of(4), "Buffer was accidentally aligned!");

        let hdr_read = unsafe { ptr::read_unaligned(misaligned_ptr) };
        let hdr_final = hdr_read.from_be();

        // Copy fields to local variables to avoid taking references to packed fields
        let cmd = hdr_final.cmd;
        let len = hdr_final.len;
        let id = hdr_final.id;
        let flags = hdr_final.flags;
        let dev = hdr_final.dev;

        assert_eq!(cmd, 0x11223344);
        assert_eq!(len, 0x55667788);
        assert_eq!(id, 0x99AABBCC);
        assert_eq!(flags, 0xDDEEFF00);
        assert_eq!(dev, 0x12345678);
    }

    #[test]
    fn test_unaligned_busaccess_read() {
        #[repr(C, align(8))]
        struct AlignedBuf([u8; 128]);
        let mut buf_wrapper = AlignedBuf([0u8; 128]);
        let buf = &mut buf_wrapper.0;
        let pkt = RpPktBusaccess {
            hdr: RpPktHdr { cmd: RpCmd::Read as u32, len: 0, id: 1, flags: 0, dev: 0 },
            timestamp: 0x1122334455667788,
            attributes: 0x99AABBCCDDEEFF00,
            addr: 0xAAAABBBBCCCCDDDD,
            len: 4,
            width: 2,
            stream_width: 1,
            master_id: 0x1234,
        }
        .to_be();

        let pkt_ptr = &pkt as *const RpPktBusaccess as *const u8;
        unsafe {
            ptr::copy_nonoverlapping(
                pkt_ptr,
                buf.as_mut_ptr().add(1),
                core::mem::size_of::<RpPktBusaccess>(),
            );
        }

        let misaligned_ptr = unsafe { buf.as_ptr().add(1) } as *const RpPktBusaccess;
        assert!(!(misaligned_ptr as usize).is_multiple_of(4), "Buffer was accidentally aligned!");

        let pkt_read = unsafe { ptr::read_unaligned(misaligned_ptr) };
        let pkt_final = pkt_read.from_be();

        let timestamp = pkt_final.timestamp;
        let addr = pkt_final.addr;
        let master_id = pkt_final.master_id;

        assert_eq!(timestamp, 0x1122334455667788);
        assert_eq!(addr, 0xAAAABBBBCCCCDDDD);
        assert_eq!(master_id, 0x1234);
    }

    #[test]
    fn test_unaligned_interrupt_read() {
        #[repr(C, align(8))]
        struct AlignedBuf([u8; 64]);
        let mut buf_wrapper = AlignedBuf([0u8; 64]);
        let buf = &mut buf_wrapper.0;
        let pkt = RpPktInterrupt {
            hdr: RpPktHdr { cmd: RpCmd::Interrupt as u32, len: 0, id: 1, flags: 0, dev: 0 },
            timestamp: 0x1122334455667788,
            vector: 0x99AABBCCDDEEFF00,
            line: 7,
            val: 1,
        }
        .to_be();

        let pkt_ptr = &pkt as *const RpPktInterrupt as *const u8;
        unsafe {
            ptr::copy_nonoverlapping(
                pkt_ptr,
                buf.as_mut_ptr().add(1),
                core::mem::size_of::<RpPktInterrupt>(),
            );
        }

        let misaligned_ptr = unsafe { buf.as_ptr().add(1) } as *const RpPktInterrupt;
        assert!(!(misaligned_ptr as usize).is_multiple_of(4), "Buffer was accidentally aligned!");

        let pkt_read = unsafe { ptr::read_unaligned(misaligned_ptr) };
        let pkt_final = pkt_read.from_be();

        let timestamp = pkt_final.timestamp;
        let line = pkt_final.line;
        let val = pkt_final.val;

        assert_eq!(timestamp, 0x1122334455667788);
        assert_eq!(line, 7);
        assert_eq!(val, 1);
    }

    #[test]
    fn test_pack_be_busaccess_byte_exact() {
        let pkt = RpPktBusaccess {
            hdr: RpPktHdr { cmd: 3, len: 38, id: 7, flags: 0, dev: 0 },
            timestamp: 0x0102030405060708,
            attributes: 0x090A0B0C0D0E0F10,
            addr: 0x1112131415161718,
            len: 4,
            width: 4,
            stream_width: 4,
            master_id: 0xABCD,
        };
        let b = pkt.pack_be();
        // hdr (20 bytes, big-endian)
        assert_eq!(&b[0..4], &3u32.to_be_bytes());
        assert_eq!(&b[4..8], &38u32.to_be_bytes());
        assert_eq!(&b[8..12], &7u32.to_be_bytes());
        assert_eq!(&b[12..16], &0u32.to_be_bytes());
        assert_eq!(&b[16..20], &0u32.to_be_bytes());
        // timestamp
        assert_eq!(&b[20..28], &0x0102030405060708u64.to_be_bytes());
        // attributes
        assert_eq!(&b[28..36], &0x090A0B0C0D0E0F10u64.to_be_bytes());
        // addr
        assert_eq!(&b[36..44], &0x1112131415161718u64.to_be_bytes());
        // len, width, stream_width
        assert_eq!(&b[44..48], &4u32.to_be_bytes());
        assert_eq!(&b[48..52], &4u32.to_be_bytes());
        assert_eq!(&b[52..56], &4u32.to_be_bytes());
        // master_id
        assert_eq!(&b[56..58], &0xABCDu16.to_be_bytes());
        assert_eq!(b.len(), 58);
    }

    #[test]
    fn test_pack_be_interrupt_byte_exact() {
        let pkt = RpPktInterrupt {
            hdr: RpPktHdr { cmd: 5, len: 21, id: 99, flags: 2, dev: 1 },
            timestamp: 0xDEADBEEFCAFEBABE,
            vector: 0x0000000000000001,
            line: 7,
            val: 1,
        };
        let b = pkt.pack_be();
        assert_eq!(&b[0..4], &5u32.to_be_bytes());
        assert_eq!(&b[4..8], &21u32.to_be_bytes());
        assert_eq!(&b[8..12], &99u32.to_be_bytes());
        assert_eq!(&b[12..16], &2u32.to_be_bytes());
        assert_eq!(&b[16..20], &1u32.to_be_bytes());
        assert_eq!(&b[20..28], &0xDEADBEEFCAFEBABEu64.to_be_bytes());
        assert_eq!(&b[28..36], &1u64.to_be_bytes());
        assert_eq!(&b[36..40], &7u32.to_be_bytes());
        assert_eq!(b[40], 1u8);
        assert_eq!(b.len(), 41);
    }

    // Stubs for linker when running tests
    #[no_mangle]
    pub extern "C" fn qemu_set_irq(_irq: QemuIrq, _level: i32) {}
    #[no_mangle]
    pub extern "C" fn memory_region_init_io(
        _mr: *mut MemoryRegion,
        _owner: *mut Object,
        _ops: *const MemoryRegionOps,
        _opaque: *mut c_void,
        _name: *const c_char,
        _size: u64,
    ) {
    }
    #[no_mangle]
    pub extern "C" fn sysbus_init_mmio(_sbd: *mut SysBusDevice, _mr: *mut MemoryRegion) {}
    #[no_mangle]
    pub extern "C" fn sysbus_mmio_map(_sbd: *mut SysBusDevice, _n: i32, _addr: u64) {}
    #[no_mangle]
    pub extern "C" fn sysbus_init_irq(_sbd: *mut SysBusDevice, _irq: *mut QemuIrq) {}
    #[no_mangle]
    pub extern "C" fn object_class_dynamic_cast_assert(
        _klass: *mut ObjectClass,
        _typename: *const c_char,
        _file: *const c_char,
        _line: i32,
        _func: *const c_char,
    ) -> *mut c_void {
        _klass as *mut c_void
    }
    #[no_mangle]
    pub extern "C" fn device_class_set_props_n(
        _dc: *mut c_void,
        _props: *const Property,
        _n: usize,
    ) {
    }
    #[no_mangle]
    pub extern "C" fn register_dso_module_init(_init: extern "C" fn(), _type: i32) {}
    #[no_mangle]
    pub extern "C" fn type_register_static(_info: *const TypeInfo) {}
    #[no_mangle]
    pub extern "C" fn virtmcu_is_bql_locked() -> bool {
        true
    }
    #[no_mangle]
    pub extern "C" fn virtmcu_safe_bql_force_lock() {}
    #[no_mangle]
    pub extern "C" fn virtmcu_safe_bql_lock() {}
    #[no_mangle]
    pub extern "C" fn virtmcu_safe_bql_unlock() {}
    #[no_mangle]
    pub extern "C" fn qemu_mutex_lock_func(_m: *mut c_void, _f: *const c_char, _l: i32) {}
    #[no_mangle]
    pub extern "C" fn qemu_mutex_unlock_impl(_m: *mut c_void, _f: *const c_char, _l: i32) {}
    #[no_mangle]
    pub extern "C" fn g_malloc0_n(_n: usize, _s: usize) -> *mut c_void {
        core::ptr::null_mut()
    }
    #[no_mangle]
    pub extern "C" fn qemu_mutex_init(_m: *mut c_void) {}
    #[no_mangle]
    pub extern "C" fn qemu_mutex_destroy(_m: *mut c_void) {}
    #[no_mangle]
    pub extern "C" fn g_free(_p: *mut c_void) {}
    #[no_mangle]
    pub extern "C" fn qemu_cond_timedwait_func(
        _c: *mut c_void,
        _m: *mut c_void,
        _t: i32,
        _f: *const c_char,
        _l: i32,
    ) -> i32 {
        1
    }
    #[no_mangle]
    pub extern "C" fn qemu_cond_broadcast(_c: *mut c_void) {}
    #[no_mangle]
    pub extern "C" fn qemu_cond_init(_c: *mut c_void) {}
    #[no_mangle]
    pub extern "C" fn qemu_cond_destroy(_c: *mut c_void) {}
    #[no_mangle]
    pub extern "C" fn error_setg_internal(
        _errp: *mut *mut c_void,
        _src: *const c_char,
        _line: i32,
        _func: *const c_char,
        _fmt: *const c_char,
    ) {
    }
    #[no_mangle]
    pub extern "C" fn qdev_prop_string() {}
    #[no_mangle]
    pub extern "C" fn qdev_prop_uint32() {}
    #[no_mangle]
    pub extern "C" fn qdev_prop_uint64() {}
}

// --- QOM Device Implementation ---

#[repr(C)]
pub struct RemotePortBridgeQEMU {
    pub parent_obj: SysBusDevice,
    pub mmio: MemoryRegion,

    pub id: *mut c_char,
    pub socket_path: *mut c_char,
    pub region_size: u32,
    pub base_addr: u64,
    pub reconnect_ms: u32,
    pub debug: bool,

    pub irqs: [QemuIrq; 32],

    pub rust_state: *mut RemotePortBridgeState,
    pub mapped: bool,
}

static MAPPED_IDS: std::sync::Mutex<Option<std::collections::HashMap<String, bool>>> =
    std::sync::Mutex::new(None);

fn is_id_mapped(id: &str) -> bool {
    let mut lock = MAPPED_IDS.lock().unwrap();
    *lock.get_or_insert_with(std::collections::HashMap::new).get(id).unwrap_or(&false)
}

fn set_id_mapped(id: &str, mapped: bool) {
    let mut lock = MAPPED_IDS.lock().unwrap();
    lock.get_or_insert_with(std::collections::HashMap::new).insert(id.to_owned(), mapped);
}

struct RawIrqArray(*mut QemuIrq);
// SAFETY: the IRQ array lives in RemotePortBridgeQEMU which outlives the transport.
// qemu_set_irq is only called while holding the BQL.
unsafe impl Send for RawIrqArray {}
unsafe impl Sync for RawIrqArray {}

pub struct RpRequest {
    pub cmd: RpCmd,
    pub addr: u64,
    pub size: u32,
    pub data: Option<[u8; 8]>,
    pub data_len: u32,
}

pub struct RpResponse {
    pub pkt: RpPktBusaccess,
    pub data: [u8; 8],
}

struct RpTransport {
    socket_path: String,
    reconnect_ms: u32,
    irqs: RawIrqArray,
    stream: Mutex<Option<UnixStream>>,
    next_id: std::sync::atomic::AtomicU32,
}

impl CoSimTransport for RpTransport {
    type Request = RpRequest;
    type Response = RpResponse;

    fn run_rx_loop(&self, ctx: &CoSimContext<Self::Response>) {
        let mut rx_buf = Vec::with_capacity(4096);
        loop {
            if !ctx.is_running() {
                break;
            }

            let stream_res = UnixStream::connect(&self.socket_path);
            let mut stream = match stream_res {
                Ok(s) => s,
                Err(_e) => {
                    if self.reconnect_ms > 0 {
                        std::thread::sleep /* SLEEP_EXCEPTION: Reconnect delay in background thread */(Duration::from_millis(self.reconnect_ms as u64));
                        continue;
                    } else {
                        virtmcu_qom::sim_err!(
                            "failed to connect to {}, exiting thread",
                            self.socket_path
                        );
                        break;
                    }
                }
            };

            // Handshake
            let hello = RpPktHello {
                hdr: RpPktHdr {
                    cmd: RpCmd::Hello as u32,
                    len: (core::mem::size_of::<RpVersion>()
                        + core::mem::size_of::<RpCapabilities>()) as u32,
                    id: 0,
                    flags: 0,
                    dev: 0,
                },
                version: RpVersion { major: RP_VERSION_MAJOR, minor: RP_VERSION_MINOR },
                caps: RpCapabilities {
                    offset: (core::mem::size_of::<RpPktHello>() as u32),
                    len: 0,
                    reserved0: 0,
                },
            };

            if stream.write_all(&hello.pack_be()).is_err() {
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

            // Read loop
            let mut temp_buf = [0u8; 1024];
            loop {
                match read_stream.read(&mut temp_buf) {
                    Ok(0) => break, // EOF
                    Ok(n) => {
                        rx_buf.extend_from_slice(&temp_buf[..n]);
                        while rx_buf.len() >= core::mem::size_of::<RpPktHdr>() {
                            let hdr_be =
                                unsafe { ptr::read_unaligned(rx_buf.as_ptr() as *const RpPktHdr) };
                            let hdr = hdr_be.from_be();
                            let pkt_len = core::mem::size_of::<RpPktHdr>() + hdr.len as usize;

                            if rx_buf.len() < pkt_len {
                                break;
                            }

                            self.handle_packet(&rx_buf[..pkt_len], &hdr, ctx);
                            rx_buf.drain(..pkt_len);
                        }
                    }
                    Err(_) => break,
                }
            }

            {
                let mut lock = self.stream.lock().unwrap();
                *lock = None;
                virtmcu_qom::sim_info!("remote disconnected");
            }
            ctx.notify_disconnected();

            if self.reconnect_ms == 0 {
                break;
            }
            std::thread::sleep /* SLEEP_EXCEPTION: Reconnect delay in background thread */(Duration::from_millis(self.reconnect_ms as u64));
        }
    }

    fn send_request(&self, req: Self::Request) -> bool {
        let mut lock = self.stream.lock().unwrap();
        if let Some(s) = lock.as_mut() {
            let id = self.next_id.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let bus_hdr_len =
                (core::mem::size_of::<RpPktBusaccess>() - core::mem::size_of::<RpPktHdr>()) as u32;
            let pkt = RpPktBusaccess {
                hdr: RpPktHdr {
                    cmd: req.cmd as u32,
                    len: bus_hdr_len + req.data_len,
                    id,
                    flags: 0,
                    dev: 0,
                },
                timestamp: 0,
                attributes: 0,
                addr: req.addr,
                len: req.size,
                width: req.size,
                stream_width: req.size,
                master_id: 0,
            };

            let pkt_bytes = pkt.pack_be();
            if s.write_all(&pkt_bytes).is_err() {
                return false;
            }
            if let Some(d) = req.data {
                if s.write_all(&d[..(req.data_len as usize)]).is_err() {
                    return false;
                }
            }
            true
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

impl RpTransport {
    fn handle_packet(&self, data: &[u8], hdr: &RpPktHdr, ctx: &CoSimContext<RpResponse>) {
        if hdr.cmd == RpCmd::Interrupt as u32 {
            if data.len() >= core::mem::size_of::<RpPktInterrupt>() {
                let pkt_be = unsafe { ptr::read_unaligned(data.as_ptr() as *const RpPktInterrupt) };
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
        } else if (hdr.cmd == RpCmd::Read as u32 || hdr.cmd == RpCmd::Write as u32)
            && data.len() >= core::mem::size_of::<RpPktBusaccess>()
        {
            let pkt_be = unsafe { ptr::read_unaligned(data.as_ptr() as *const RpPktBusaccess) };
            let pkt = pkt_be.from_be();

            let bus_hdr_len =
                core::mem::size_of::<RpPktBusaccess>() - core::mem::size_of::<RpPktHdr>();
            let payload_len = hdr.len as usize - bus_hdr_len;
            let mut resp_data = [0u8; 8];
            if payload_len > 0 && payload_len <= 8 {
                resp_data[..payload_len].copy_from_slice(
                    &data[core::mem::size_of::<RpPktBusaccess>()
                        ..core::mem::size_of::<RpPktBusaccess>() + payload_len],
                );
            }
            ctx.dispatch_response(RpResponse { pkt, data: resp_data });
        }
    }
}

pub struct RemotePortBridgeState {
    bridge: CoSimBridge<RpTransport>,
}

unsafe extern "C" fn bridge_read(opaque: *mut c_void, addr: u64, size: c_uint) -> u64 {
    let qemu = unsafe { &*(opaque as *mut RemotePortBridgeQEMU) };
    if qemu.debug {
        virtmcu_qom::sim_warn!("remote_port_read: addr=0x{:x} size={}", addr, size);
    }
    let state = &*qemu.rust_state;
    let req = RpRequest { cmd: RpCmd::Read, addr, size, data: None, data_len: 0 };

    state.bridge.wait_connected(5000);

    if let Some(resp) = state.bridge.send_and_wait(req, 5000) {
        if size <= 8 {
            let mut buf = [0u8; 8];
            buf[..size as usize].copy_from_slice(&resp.data[..size as usize]);
            u64::from_le_bytes(buf)
        } else {
            0
        }
    } else {
        0
    }
}

unsafe extern "C" fn bridge_write(opaque: *mut c_void, addr: u64, val: u64, size: c_uint) {
    let qemu = unsafe { &*(opaque as *mut RemotePortBridgeQEMU) };
    if qemu.debug {
        virtmcu_qom::sim_warn!(
            "remote_port_write: addr=0x{:x} val=0x{:x} size={}",
            addr,
            val,
            size
        );
    }
    let state = &*qemu.rust_state;
    let val_bytes = val.to_le_bytes();
    let req = RpRequest { cmd: RpCmd::Write, addr, size, data: Some(val_bytes), data_len: size };

    state.bridge.wait_connected(5000);
    state.bridge.send_and_wait(req, 5000);
}

static BRIDGE_MMIO_OPS: MemoryRegionOps = MemoryRegionOps {
    read: Some(bridge_read),
    write: Some(bridge_write),
    read_with_attrs: ptr::null(),
    write_with_attrs: ptr::null(),
    endianness: DEVICE_NATIVE_ENDIAN,
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
    let qemu = &mut *(dev as *mut RemotePortBridgeQEMU);
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

    let transport = RpTransport {
        socket_path: CStr::from_ptr(qemu.socket_path).to_string_lossy().into_owned(),
        reconnect_ms: qemu.reconnect_ms,
        irqs: RawIrqArray(qemu.irqs.as_mut_ptr()),
        stream: Mutex::new(None),
        next_id: std::sync::atomic::AtomicU32::new(0),
    };

    let bridge = CoSimBridge::new(transport);
    let state = Box::new(RemotePortBridgeState { bridge });
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
            dev,
            c"remote-port-bridge".as_ptr(),
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
    let qemu = &mut *(obj as *mut RemotePortBridgeQEMU);
    if !qemu.rust_state.is_null() {
        let state = Box::from_raw(qemu.rust_state);
        drop(state); // CoSimBridge handles safe Drop teardown + vCPU drain

        if qemu.mapped && !qemu.id.is_null() {
            let id = CStr::from_ptr(qemu.id).to_string_lossy().into_owned();
            set_id_mapped(&id, false);
        }

        qemu.rust_state = ptr::null_mut();
    }
}

unsafe extern "C" fn bridge_unrealize(_dev: *mut c_void) {}

static BRIDGE_PROPERTIES: [Property; 7] = [
    define_prop_string!(c"id".as_ptr(), RemotePortBridgeQEMU, id),
    define_prop_string!(c"socket-path".as_ptr(), RemotePortBridgeQEMU, socket_path),
    define_prop_uint32!(c"region-size".as_ptr(), RemotePortBridgeQEMU, region_size, 0x1000),
    define_prop_uint64!(c"base-addr".as_ptr(), RemotePortBridgeQEMU, base_addr, u64::MAX),
    define_prop_uint32!(c"reconnect-ms".as_ptr(), RemotePortBridgeQEMU, reconnect_ms, 1000),
    virtmcu_qom::define_prop_bool!(c"debug".as_ptr(), RemotePortBridgeQEMU, debug, false),
    unsafe { core::mem::zeroed() },
];

unsafe extern "C" fn bridge_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let dc = device_class!(klass);
    (*dc).realize = Some(bridge_realize);
    (*dc).unrealize = Some(bridge_unrealize);
    (*dc).user_creatable = true;
    virtmcu_qom::qdev::device_class_set_props_n(dc, BRIDGE_PROPERTIES.as_ptr(), 6);
}

#[used]
static BRIDGE_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"remote-port-bridge".as_ptr(),
    parent: c"sys-bus-device".as_ptr(),
    instance_size: core::mem::size_of::<RemotePortBridgeQEMU>(),
    instance_align: 0,
    instance_init: Some(bridge_instance_init),
    instance_post_init: None,
    instance_finalize: Some(bridge_instance_finalize),
    abstract_: false,
    class_size: core::mem::size_of::<virtmcu_qom::qdev::SysBusDeviceClass>(),
    class_init: Some(bridge_class_init),
    class_base_init: None,
    class_data: ptr::null(),
    interfaces: ptr::null(),
};

declare_device_type!(REMOTE_PORT_BRIDGE_TYPE_INIT, BRIDGE_TYPE_INFO);
