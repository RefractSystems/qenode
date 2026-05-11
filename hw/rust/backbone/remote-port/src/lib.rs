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

const RP_MAX_DATA_SIZE: usize = 8;
const RP_RX_BUF_SIZE: usize = 4096;
const RP_TEMP_BUF_SIZE: usize = 1024;
const RP_COSIM_TIMEOUT_MS: u32 = 5000;
const RP_MAX_IRQS: usize = 32;
const RP_BUSACCESS_PKT_SIZE: usize = 58;
const RP_INTERRUPT_PKT_SIZE: usize = 41;

pub const RP_VERSION_MAJOR: u16 = 4;
pub const RP_VERSION_MINOR: u16 = 3;

pub const RP_PKT_HDR_SIZE: usize = 20;
pub const RP_VERSION_SIZE: usize = 4;
pub const RP_CAPS_SIZE: usize = 8;
pub const RP_PKT_HELLO_SIZE: usize = 32;

// Field sizes
const RP_U64_SIZE: usize = 8;
const RP_U32_SIZE: usize = 4;
const RP_U16_SIZE: usize = 2;

// RpPktHdr field offsets
const RP_HDR_OFF_CMD: usize = 0;
const RP_HDR_OFF_LEN: usize = 4;
const RP_HDR_OFF_ID: usize = 8;
const RP_HDR_OFF_FLAGS: usize = 12;
const RP_HDR_OFF_DEV: usize = 16;

// RpVersion field offsets
const RP_VERSION_OFF_MAJOR: usize = 0;
const RP_VERSION_OFF_MINOR: usize = 2;

// RpCapabilities field offsets
const RP_CAPS_OFF_OFFSET: usize = 0;
const RP_CAPS_OFF_LEN: usize = 4;
const RP_CAPS_OFF_RESERVED: usize = 6;

// RpPktBusaccess field offsets
const RP_BUSACCESS_OFF_TIMESTAMP: usize = 20;
const RP_BUSACCESS_OFF_ATTRIBUTES: usize = 28;
const RP_BUSACCESS_OFF_ADDR: usize = 36;
const RP_BUSACCESS_OFF_LEN: usize = 44;
const RP_BUSACCESS_OFF_WIDTH: usize = 48;
const RP_BUSACCESS_OFF_STREAM_WIDTH: usize = 52;
const RP_BUSACCESS_OFF_MASTER_ID: usize = 56;

// RpPktInterrupt field offsets
const RP_INTERRUPT_OFF_TIMESTAMP: usize = 20;
const RP_INTERRUPT_OFF_VECTOR: usize = 28;
const RP_INTERRUPT_OFF_LINE: usize = 36;
const RP_INTERRUPT_OFF_VAL: usize = 40;

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
    pub fn pack_be(&self) -> [u8; RP_PKT_HDR_SIZE] {
        let mut b = [0u8; RP_PKT_HDR_SIZE];
        b[RP_HDR_OFF_CMD..RP_HDR_OFF_CMD + RP_U32_SIZE].copy_from_slice(&self.cmd.to_be_bytes());
        b[RP_HDR_OFF_LEN..RP_HDR_OFF_LEN + RP_U32_SIZE].copy_from_slice(&self.len.to_be_bytes());
        b[RP_HDR_OFF_ID..RP_HDR_OFF_ID + RP_U32_SIZE].copy_from_slice(&self.id.to_be_bytes());
        b[RP_HDR_OFF_FLAGS..RP_HDR_OFF_FLAGS + RP_U32_SIZE]
            .copy_from_slice(&self.flags.to_be_bytes());
        b[RP_HDR_OFF_DEV..RP_HDR_OFF_DEV + RP_U32_SIZE].copy_from_slice(&self.dev.to_be_bytes());
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
    pub fn pack_be(&self) -> [u8; RP_VERSION_SIZE] {
        let mut b = [0u8; RP_VERSION_SIZE];
        b[RP_VERSION_OFF_MAJOR..RP_VERSION_OFF_MAJOR + RP_U16_SIZE]
            .copy_from_slice(&self.major.to_be_bytes());
        b[RP_VERSION_OFF_MINOR..RP_VERSION_OFF_MINOR + RP_U16_SIZE]
            .copy_from_slice(&self.minor.to_be_bytes());
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
    pub fn pack_be(&self) -> [u8; RP_CAPS_SIZE] {
        let mut b = [0u8; RP_CAPS_SIZE];
        b[RP_CAPS_OFF_OFFSET..RP_CAPS_OFF_OFFSET + RP_U32_SIZE]
            .copy_from_slice(&self.offset.to_be_bytes());
        b[RP_CAPS_OFF_LEN..RP_CAPS_OFF_LEN + RP_U16_SIZE].copy_from_slice(&self.len.to_be_bytes());
        b[RP_CAPS_OFF_RESERVED..RP_CAPS_OFF_RESERVED + RP_U16_SIZE]
            .copy_from_slice(&self.reserved0.to_be_bytes());
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
    pub fn pack_be(&self) -> [u8; RP_PKT_HELLO_SIZE] {
        let mut b = [0u8; RP_PKT_HELLO_SIZE];
        b[0..RP_PKT_HDR_SIZE].copy_from_slice(&self.hdr.pack_be());
        b[RP_PKT_HDR_SIZE..RP_PKT_HDR_SIZE + RP_VERSION_SIZE]
            .copy_from_slice(&self.version.pack_be());
        b[RP_PKT_HDR_SIZE + RP_VERSION_SIZE..RP_PKT_HELLO_SIZE]
            .copy_from_slice(&self.caps.pack_be());
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
    pub fn pack_be(&self) -> [u8; RP_BUSACCESS_PKT_SIZE] {
        let mut b = [0u8; RP_BUSACCESS_PKT_SIZE];
        b[0..RP_PKT_HDR_SIZE].copy_from_slice(&self.hdr.pack_be());
        b[RP_BUSACCESS_OFF_TIMESTAMP..RP_BUSACCESS_OFF_TIMESTAMP + RP_U64_SIZE]
            .copy_from_slice(&self.timestamp.to_be_bytes());
        b[RP_BUSACCESS_OFF_ATTRIBUTES..RP_BUSACCESS_OFF_ATTRIBUTES + RP_U64_SIZE]
            .copy_from_slice(&self.attributes.to_be_bytes());
        b[RP_BUSACCESS_OFF_ADDR..RP_BUSACCESS_OFF_ADDR + RP_U64_SIZE]
            .copy_from_slice(&self.addr.to_be_bytes());
        b[RP_BUSACCESS_OFF_LEN..RP_BUSACCESS_OFF_LEN + RP_U32_SIZE]
            .copy_from_slice(&self.len.to_be_bytes());
        b[RP_BUSACCESS_OFF_WIDTH..RP_BUSACCESS_OFF_WIDTH + RP_U32_SIZE]
            .copy_from_slice(&self.width.to_be_bytes());
        b[RP_BUSACCESS_OFF_STREAM_WIDTH..RP_BUSACCESS_OFF_STREAM_WIDTH + RP_U32_SIZE]
            .copy_from_slice(&self.stream_width.to_be_bytes());
        b[RP_BUSACCESS_OFF_MASTER_ID..RP_BUSACCESS_OFF_MASTER_ID + RP_U16_SIZE]
            .copy_from_slice(&self.master_id.to_be_bytes());
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
    pub fn pack_be(&self) -> [u8; RP_INTERRUPT_PKT_SIZE] {
        let mut b = [0u8; RP_INTERRUPT_PKT_SIZE];
        b[0..RP_PKT_HDR_SIZE].copy_from_slice(&self.hdr.pack_be());
        b[RP_INTERRUPT_OFF_TIMESTAMP..RP_INTERRUPT_OFF_TIMESTAMP + RP_U64_SIZE]
            .copy_from_slice(&self.timestamp.to_be_bytes());
        b[RP_INTERRUPT_OFF_VECTOR..RP_INTERRUPT_OFF_VECTOR + RP_U64_SIZE]
            .copy_from_slice(&self.vector.to_be_bytes());
        b[RP_INTERRUPT_OFF_LINE..RP_INTERRUPT_OFF_LINE + RP_U32_SIZE]
            .copy_from_slice(&self.line.to_be_bytes());
        b[RP_INTERRUPT_OFF_VAL] = self.val;
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
#[allow(clippy::magic_numbers)] // virtmcu-allow: allow reasoning="Tests require specific magic numbers"
mod tests {
    use super::*;
    use core::ptr;

    #[test]
    fn test_unaligned_hdr_read() {
        const TEST_CMD: u32 = 0x11223344;
        const TEST_LEN: u32 = 0x55667788;
        const TEST_ID: u32 = 0x99AABBCC;
        const TEST_FLAGS: u32 = 0xDDEEFF00;
        const TEST_DEV: u32 = 0x12345678;

        const _: () = ();
        #[repr(C, align(8))] // virtmcu-allow: align requirements
        struct AlignedBuf([u8; 32]);
        let mut buf_wrapper = AlignedBuf([0u8; 32]);
        let buf = &mut buf_wrapper.0;
        let hdr = RpPktHdr {
            cmd: TEST_CMD,
            len: TEST_LEN,
            id: TEST_ID,
            flags: TEST_FLAGS,
            dev: TEST_DEV,
        }
        .to_be();

        // Write at offset 1 to force misalignment
        let hdr_ptr = &hdr as *const RpPktHdr as *const u8;
        unsafe {
            let n = core::mem::size_of::<RpPktHdr>();
            ptr::copy_nonoverlapping(hdr_ptr, buf.as_mut_ptr().add(1), n); // virtmcu-allow: copy reasoning="test"
        }

        let misaligned_ptr = unsafe { buf.as_ptr().add(1) } as *const RpPktHdr;
        const ALIGN_4: usize = 4;
        assert!(
            !(misaligned_ptr as usize).is_multiple_of(ALIGN_4),
            "Buffer was accidentally aligned!"
        );

        let hdr_read = unsafe { ptr::read_unaligned(misaligned_ptr) };
        let hdr_final = hdr_read.from_be();

        // Copy fields to local variables to avoid taking references to packed fields
        let cmd = hdr_final.cmd;
        let len = hdr_final.len;
        let id = hdr_final.id;
        let flags = hdr_final.flags;
        let dev = hdr_final.dev;

        assert_eq!(cmd, TEST_CMD);
        assert_eq!(len, TEST_LEN);
        assert_eq!(id, TEST_ID);
        assert_eq!(flags, TEST_FLAGS);
        assert_eq!(dev, TEST_DEV);
    }

    #[test]
    fn test_unaligned_busaccess_read() {
        const TEST_TS: u64 = 0x1122334455667788;
        const TEST_ATTR: u64 = 0x99AABBCCDDEEFF00;
        const TEST_ADDR: u64 = 0xAAAABBBBCCCCDDDD;
        const TEST_LEN: u32 = 4;
        const TEST_WIDTH: u32 = 2;
        const TEST_MASTER: u16 = 0x1234;

        const _: () = ();
        #[repr(C, align(8))] // virtmcu-allow: align requirements
        struct AlignedBuf([u8; 128]);
        let mut buf_wrapper = AlignedBuf([0u8; 128]);
        let buf = &mut buf_wrapper.0;
        let pkt = RpPktBusaccess {
            hdr: RpPktHdr { cmd: RpCmd::Read as u32, len: 0, id: 1, flags: 0, dev: 0 },
            timestamp: TEST_TS,
            attributes: TEST_ATTR,
            addr: TEST_ADDR,
            len: TEST_LEN,
            width: TEST_WIDTH,
            stream_width: 1,
            master_id: TEST_MASTER,
        }
        .to_be();

        let pkt_ptr = &pkt as *const RpPktBusaccess as *const u8;
        unsafe {
            let n = core::mem::size_of::<RpPktBusaccess>();
            ptr::copy_nonoverlapping(pkt_ptr, buf.as_mut_ptr().add(1), n); // virtmcu-allow: copy reasoning="test"
        }

        let misaligned_ptr = unsafe { buf.as_ptr().add(1) } as *const RpPktBusaccess;
        const ALIGN_4: usize = 4;
        assert!(
            !(misaligned_ptr as usize).is_multiple_of(ALIGN_4),
            "Buffer was accidentally aligned!"
        );

        let pkt_read = unsafe { ptr::read_unaligned(misaligned_ptr) };
        let pkt_final = pkt_read.from_be();

        let timestamp = pkt_final.timestamp;
        let addr = pkt_final.addr;
        let master_id = pkt_final.master_id;

        assert_eq!(timestamp, TEST_TS);
        assert_eq!(addr, TEST_ADDR);
        assert_eq!(master_id, TEST_MASTER);
    }

    #[test]
    fn test_unaligned_interrupt_read() {
        const TEST_TS_INT: u64 = 0x1122334455667788;
        const TEST_VEC_INT: u64 = 0x99AABBCCDDEEFF00;
        const TEST_LINE_7: u32 = 7;
        const TEST_VAL_1: u32 = 1;
        const _: () = ();
        #[repr(C, align(8))] // virtmcu-allow: align requirements
        struct AlignedBuf([u8; 64]);
        let mut buf_wrapper = AlignedBuf([0u8; 64]);
        let buf = &mut buf_wrapper.0;
        let pkt = RpPktInterrupt {
            hdr: RpPktHdr { cmd: RpCmd::Interrupt as u32, len: 0, id: 1, flags: 0, dev: 0 },
            timestamp: TEST_TS_INT,
            vector: TEST_VEC_INT,
            line: TEST_LINE_7,
            val: TEST_VAL_1,
        }
        .to_be();

        let pkt_ptr = &pkt as *const RpPktInterrupt as *const u8;
        unsafe {
            let n = core::mem::size_of::<RpPktInterrupt>();
            ptr::copy_nonoverlapping(hdr_ptr, buf.as_mut_ptr().add(1), n); // virtmcu-allow: copy reasoning="test"
        }

        let misaligned_ptr = unsafe { buf.as_ptr().add(1) } as *const RpPktInterrupt;
        const ALIGN_4: usize = 4;
        assert!(
            !(misaligned_ptr as usize).is_multiple_of(ALIGN_4),
            "Buffer was accidentally aligned!"
        );

        let hdr_read = unsafe { ptr::read_unaligned(misaligned_ptr) };
        let hdr_final = hdr_read.from_be();

        let timestamp = hdr_final.timestamp;
        let line = hdr_final.line;
        let val = hdr_final.val;

        assert_eq!(timestamp, TEST_TS_INT);
        assert_eq!(line, TEST_LINE_7);
        assert_eq!(val, TEST_VAL_1);
    }

    #[test]
    fn test_pack_be_busaccess_byte_exact() {
        const TEST_CMD_3: u32 = 3;
        const TEST_LEN_38: u32 = 38;
        const TEST_ID_7: u32 = 7;
        const TEST_TS_PACK: u64 = 0x0102030405060708;
        const TEST_ATTR_PACK: u64 = 0x090A0B0C0D0E0F10;
        const TEST_ADDR_PACK: u64 = 0x1112131415161718;
        const TEST_SIZE_4: u32 = 4;
        const TEST_MASTER_0XABCD: u16 = 0xABCD;
        const EXPECTED_LEN: usize = 58;

        let pkt = RpPktBusaccess {
            hdr: RpPktHdr { cmd: TEST_CMD_3, len: TEST_LEN_38, id: TEST_ID_7, flags: 0, dev: 0 },
            timestamp: TEST_TS_PACK,
            attributes: TEST_ATTR_PACK,
            addr: TEST_ADDR_PACK,
            len: TEST_SIZE_4,
            width: TEST_SIZE_4,
            stream_width: TEST_SIZE_4,
            master_id: TEST_MASTER_0XABCD,
        };
        let b = pkt.pack_be();
        // hdr (20 bytes, big-endian)
        assert_eq!(&b[RP_HDR_OFF_CMD..RP_HDR_OFF_CMD + RP_U32_SIZE], &TEST_CMD_3.to_be_bytes());
        assert_eq!(&b[RP_HDR_OFF_LEN..RP_HDR_OFF_LEN + RP_U32_SIZE], &TEST_LEN_38.to_be_bytes());
        assert_eq!(&b[RP_HDR_OFF_ID..RP_HDR_OFF_ID + RP_U32_SIZE], &TEST_ID_7.to_be_bytes());
        assert_eq!(&b[RP_HDR_OFF_FLAGS..RP_HDR_OFF_FLAGS + RP_U32_SIZE], &0u32.to_be_bytes());
        assert_eq!(&b[RP_HDR_OFF_DEV..RP_HDR_OFF_DEV + RP_U32_SIZE], &0u32.to_be_bytes());
        // timestamp
        assert_eq!(
            &b[RP_BUSACCESS_OFF_TIMESTAMP..RP_BUSACCESS_OFF_TIMESTAMP + RP_U64_SIZE],
            &TEST_TS_PACK.to_be_bytes()
        );
        // attributes
        assert_eq!(
            &b[RP_BUSACCESS_OFF_ATTRIBUTES..RP_BUSACCESS_OFF_ATTRIBUTES + RP_U64_SIZE],
            &TEST_ATTR_PACK.to_be_bytes()
        );
        // addr
        assert_eq!(
            &b[RP_BUSACCESS_OFF_ADDR..RP_BUSACCESS_OFF_ADDR + RP_U64_SIZE],
            &TEST_ADDR_PACK.to_be_bytes()
        );
        // len, width, stream_width
        assert_eq!(
            &b[RP_BUSACCESS_OFF_LEN..RP_BUSACCESS_OFF_LEN + RP_U32_SIZE],
            &TEST_SIZE_4.to_be_bytes()
        );
        assert_eq!(
            &b[RP_BUSACCESS_OFF_WIDTH..RP_BUSACCESS_OFF_WIDTH + RP_U32_SIZE],
            &TEST_SIZE_4.to_be_bytes()
        );
        assert_eq!(
            &b[RP_BUSACCESS_OFF_STREAM_WIDTH..RP_BUSACCESS_OFF_STREAM_WIDTH + RP_U32_SIZE],
            &TEST_SIZE_4.to_be_bytes()
        );
        // master_id
        assert_eq!(
            &b[RP_BUSACCESS_OFF_MASTER_ID..RP_BUSACCESS_OFF_MASTER_ID + RP_U16_SIZE],
            &TEST_MASTER_0XABCD.to_be_bytes()
        );
        assert_eq!(b.len(), EXPECTED_LEN);
    }

    #[test]
    fn test_pack_be_interrupt_byte_exact() {
        const TEST_CMD_5: u32 = 5;
        const TEST_LEN_21: u32 = 21;
        const TEST_ID_99: u32 = 99;
        const TEST_FLAGS_2: u32 = 2;
        const TEST_TS_INT_PACK: u64 = 0xDEADBEEFCAFEBABE;
        const TEST_VEC_1: u64 = 1;
        const TEST_LINE_7_INT: u32 = 7;
        const TEST_VAL_1_INT: u8 = 1;
        const EXPECTED_LEN_INT: usize = 41;

        let pkt = RpPktInterrupt {
            hdr: RpPktHdr {
                cmd: TEST_CMD_5,
                len: TEST_LEN_21,
                id: TEST_ID_99,
                flags: TEST_FLAGS_2,
                dev: 1,
            },
            timestamp: TEST_TS_INT_PACK,
            vector: TEST_VEC_1,
            line: TEST_LINE_7_INT,
            val: TEST_VAL_1_INT,
        };
        let b = pkt.pack_be();
        assert_eq!(&b[RP_HDR_OFF_CMD..RP_HDR_OFF_CMD + RP_U32_SIZE], &TEST_CMD_5.to_be_bytes());
        assert_eq!(&b[RP_HDR_OFF_LEN..RP_HDR_OFF_LEN + RP_U32_SIZE], &TEST_LEN_21.to_be_bytes());
        assert_eq!(&b[RP_HDR_OFF_ID..RP_HDR_OFF_ID + RP_U32_SIZE], &TEST_ID_99.to_be_bytes());
        assert_eq!(
            &b[RP_HDR_OFF_FLAGS..RP_HDR_OFF_FLAGS + RP_U32_SIZE],
            &TEST_FLAGS_2.to_be_bytes()
        );
        assert_eq!(&b[RP_HDR_OFF_DEV..RP_HDR_OFF_DEV + RP_U32_SIZE], &1u32.to_be_bytes());
        assert_eq!(
            &b[RP_INTERRUPT_OFF_TIMESTAMP..RP_INTERRUPT_OFF_TIMESTAMP + RP_U64_SIZE],
            &TEST_TS_INT_PACK.to_be_bytes()
        );
        assert_eq!(
            &b[RP_INTERRUPT_OFF_VECTOR..RP_INTERRUPT_OFF_VECTOR + RP_U64_SIZE],
            &TEST_VEC_1.to_be_bytes()
        );
        assert_eq!(
            &b[RP_INTERRUPT_OFF_LINE..RP_INTERRUPT_OFF_LINE + RP_U32_SIZE],
            &TEST_LINE_7_INT.to_be_bytes()
        );
        assert_eq!(b[RP_INTERRUPT_OFF_VAL], TEST_VAL_1_INT);
        assert_eq!(b.len(), EXPECTED_LEN_INT);
    }
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

    pub irqs: [QemuIrq; RP_MAX_IRQS],

    pub rust_state: *mut RemotePortBridgeState,
    pub mapped: bool,
}

// virtmcu-allow: static_state reasoning="Singleton state workaround for adapter registration"
static MAPPED_IDS: std::sync::Mutex<Option<std::collections::HashMap<String, bool>>> =
    std::sync::Mutex::new(None);

fn is_id_mapped(id: &str) -> bool {
    let mut lock = MAPPED_IDS.lock().expect("remote port error");
    *lock.get_or_insert_with(std::collections::HashMap::new).get(id).unwrap_or(&false)
}

fn set_id_mapped(id: &str, mapped: bool) {
    let mut lock = MAPPED_IDS.lock().expect("remote port error");
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
    pub data: Option<[u8; RP_MAX_DATA_SIZE]>,
    pub data_len: u32,
}

pub struct RpResponse {
    pub pkt: RpPktBusaccess,
    pub data: [u8; RP_MAX_DATA_SIZE],
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
        let mut rx_buf = Vec::with_capacity(RP_RX_BUF_SIZE);
        loop {
            if !ctx.is_running() {
                break;
            }

            let stream_res = UnixStream::connect(&self.socket_path);
            let mut stream = match stream_res {
                Ok(s) => s,
                Err(_e) => {
                    if self.reconnect_ms > 0 {
                        ctx.sleep_interruptible(Duration::from_millis(self.reconnect_ms as u64));
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
                let mut lock = self.stream.lock().expect("remote port error");
                *lock = Some(stream);
                virtmcu_qom::sim_info!("connected to {}", self.socket_path);
            }
            ctx.notify_connected();

            // Read loop
            let mut temp_buf = [0u8; RP_TEMP_BUF_SIZE];

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
                let mut lock = self.stream.lock().expect("remote port error");
                *lock = None;
                virtmcu_qom::sim_info!("remote disconnected");
            }
            ctx.notify_disconnected();

            if self.reconnect_ms == 0 {
                break;
            }
            ctx.sleep_interruptible(Duration::from_millis(self.reconnect_ms as u64));
        }
    }

    fn send_request(&self, req: Self::Request) -> bool {
        let mut lock = self.stream.lock().expect("remote port error");
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
        let mut lock = self.stream.lock().expect("remote port error");
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
                if pkt.line < RP_MAX_IRQS as u32 {
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
            let mut resp_data = [0u8; RP_MAX_DATA_SIZE];
            if payload_len > 0 && payload_len <= RP_MAX_DATA_SIZE {
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

    state.bridge.wait_connected(RP_COSIM_TIMEOUT_MS);

    if let Some(resp) = state.bridge.send_and_wait(req, RP_COSIM_TIMEOUT_MS) {
        if size <= RP_MAX_DATA_SIZE as u32 {
            let mut buf = [0u8; RP_MAX_DATA_SIZE];
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

    state.bridge.wait_connected(RP_COSIM_TIMEOUT_MS);
    state.bridge.send_and_wait(req, RP_COSIM_TIMEOUT_MS);
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
        max_access_size: RP_MAX_DATA_SIZE as u32,
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

    for i in 0..RP_MAX_IRQS {
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

const DEFAULT_REGION_SIZE: u32 = 0x1000;
const DEFAULT_RECONNECT_MS: u32 = 1000;
const BRIDGE_PROPERTIES_COUNT: usize = 6;

static BRIDGE_PROPERTIES: [Property; BRIDGE_PROPERTIES_COUNT] = [
    define_prop_string!(c"id".as_ptr(), RemotePortBridgeQEMU, id),
    define_prop_string!(c"socket-path".as_ptr(), RemotePortBridgeQEMU, socket_path),
    define_prop_uint32!(
        c"region-size".as_ptr(),
        RemotePortBridgeQEMU,
        region_size,
        DEFAULT_REGION_SIZE
    ),
    define_prop_uint64!(c"base-addr".as_ptr(), RemotePortBridgeQEMU, base_addr, u64::MAX),
    define_prop_uint32!(
        c"reconnect-ms".as_ptr(),
        RemotePortBridgeQEMU,
        reconnect_ms,
        DEFAULT_RECONNECT_MS
    ),
    virtmcu_qom::define_prop_bool!(c"debug".as_ptr(), RemotePortBridgeQEMU, debug, false),
];

unsafe extern "C" fn bridge_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let dc = device_class!(klass);
    (*dc).realize = Some(bridge_realize);
    (*dc).unrealize = Some(bridge_unrealize);
    (*dc).user_creatable = true;
    virtmcu_qom::qdev::device_class_set_props_n(
        dc,
        BRIDGE_PROPERTIES.as_ptr(),
        BRIDGE_PROPERTIES_COUNT,
    );
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
