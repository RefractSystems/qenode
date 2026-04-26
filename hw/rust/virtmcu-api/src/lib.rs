#![deny(missing_docs)]
#![doc = "The crate"]
#[allow(
    clippy::all,
    missing_docs,
    clippy::unwrap_used,
    clippy::missing_safety_doc,
    clippy::extra_unused_lifetimes
)]
pub mod can_generated;
#[allow(
    clippy::all,
    missing_docs,
    clippy::unwrap_used,
    clippy::missing_safety_doc,
    clippy::extra_unused_lifetimes
)]
pub mod flexray_generated;
#[allow(
    clippy::all,
    missing_docs,
    clippy::unwrap_used,
    clippy::missing_safety_doc,
    clippy::extra_unused_lifetimes
)]
pub mod lin_generated;
#[allow(
    clippy::all,
    missing_docs,
    clippy::unwrap_used,
    clippy::missing_safety_doc,
    clippy::extra_unused_lifetimes
)]
pub mod rf_generated;
#[allow(
    clippy::all,
    missing_docs,
    clippy::unwrap_used,
    clippy::missing_safety_doc,
    clippy::extra_unused_lifetimes
)]
pub mod telemetry_generated;
#[allow(
    clippy::all,
    missing_docs,
    clippy::unwrap_used,
    clippy::missing_safety_doc,
    clippy::extra_unused_lifetimes
)]
pub mod wifi_generated;

/// A constant
pub const VIRTMCU_PROTO_MAGIC: u32 = 0x564D4355;
/// A constant
pub const VIRTMCU_PROTO_VERSION: u32 = 1;

#[repr(C, packed)]
#[derive(Debug, Copy, Clone, Default)]
/// A struct
pub struct VirtmcuHandshake {
    /// A struct field
    pub magic: u32,
    /// A struct field
    pub version: u32,
}

impl VirtmcuHandshake {
    /// Pack into a byte array (little-endian).
    pub fn pack(&self) -> [u8; 8] {
        let mut b = [0u8; 8];
        b[0..4].copy_from_slice(&self.magic.to_le_bytes());
        b[4..8].copy_from_slice(&self.version.to_le_bytes());
        b
    }

    /// Unpack from a byte array (little-endian).
    pub fn unpack(b: &[u8; 8]) -> Self {
        Self {
            magic: u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            version: u32::from_le_bytes([b[4], b[5], b[6], b[7]]),
        }
    }

    /// Unpack from a byte slice. Returns None if the slice is not the correct length.
    pub fn unpack_slice(b: &[u8]) -> Option<Self> {
        b.get(0..core::mem::size_of::<Self>())?.try_into().ok().map(Self::unpack)
    }
}

/// A constant
pub const MMIO_REQ_READ: u8 = 0;
/// A constant
pub const MMIO_REQ_WRITE: u8 = 1;

#[repr(C, packed)]
#[derive(Debug, Copy, Clone, Default)]
/// A struct
pub struct MmioReq {
    /// A struct field
    pub type_: u8,
    /// A struct field
    pub size: u8,
    /// A struct field
    pub reserved1: u16,
    /// A struct field
    pub reserved2: u32,
    /// A struct field
    pub vtime_ns: u64,
    /// A struct field
    pub addr: u64,
    /// A struct field
    pub data: u64,
}

impl MmioReq {
    /// Pack into a byte array (little-endian).
    pub fn pack(&self) -> [u8; 32] {
        let mut b = [0u8; 32];
        b[0] = self.type_;
        b[1] = self.size;
        b[2..4].copy_from_slice(&self.reserved1.to_le_bytes());
        b[4..8].copy_from_slice(&self.reserved2.to_le_bytes());
        b[8..16].copy_from_slice(&self.vtime_ns.to_le_bytes());
        b[16..24].copy_from_slice(&self.addr.to_le_bytes());
        b[24..32].copy_from_slice(&self.data.to_le_bytes());
        b
    }

    /// Unpack from a byte array (little-endian).
    pub fn unpack(b: &[u8; 32]) -> Self {
        Self {
            type_: b[0],
            size: b[1],
            reserved1: u16::from_le_bytes([b[2], b[3]]),
            reserved2: u32::from_le_bytes([b[4], b[5], b[6], b[7]]),
            vtime_ns: u64::from_le_bytes([b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]]),
            addr: u64::from_le_bytes([b[16], b[17], b[18], b[19], b[20], b[21], b[22], b[23]]),
            data: u64::from_le_bytes([b[24], b[25], b[26], b[27], b[28], b[29], b[30], b[31]]),
        }
    }

    /// Unpack from a byte slice. Returns None if the slice is not the correct length.
    pub fn unpack_slice(b: &[u8]) -> Option<Self> {
        b.get(0..core::mem::size_of::<Self>())?.try_into().ok().map(Self::unpack)
    }
}

/// A constant
pub const SYSC_MSG_RESP: u32 = 0;
/// A constant
pub const SYSC_MSG_IRQ_SET: u32 = 1;
/// A constant
pub const SYSC_MSG_IRQ_CLEAR: u32 = 2;

#[repr(C, packed)]
#[derive(Debug, Copy, Clone, Default)]
/// A struct
pub struct SyscMsg {
    /// A struct field
    pub type_: u32,
    /// A struct field
    pub irq_num: u32,
    /// A struct field
    pub data: u64,
}

impl SyscMsg {
    /// Pack into a byte array (little-endian).
    pub fn pack(&self) -> [u8; 16] {
        let mut b = [0u8; 16];
        b[0..4].copy_from_slice(&self.type_.to_le_bytes());
        b[4..8].copy_from_slice(&self.irq_num.to_le_bytes());
        b[8..16].copy_from_slice(&self.data.to_le_bytes());
        b
    }

    /// Unpack from a byte array (little-endian).
    pub fn unpack(b: &[u8; 16]) -> Self {
        Self {
            type_: u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            irq_num: u32::from_le_bytes([b[4], b[5], b[6], b[7]]),
            data: u64::from_le_bytes([b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]]),
        }
    }

    /// Unpack from a byte slice. Returns None if the slice is not the correct length.
    pub fn unpack_slice(b: &[u8]) -> Option<Self> {
        b.get(0..core::mem::size_of::<Self>())?.try_into().ok().map(Self::unpack)
    }
}

/// Clock advancement request sent from TimeAuthority to the node.
#[repr(C, packed)]
#[derive(Debug, Copy, Clone, Default)]
pub struct ClockAdvanceReq {
    /// How many nanoseconds to advance the clock.
    pub delta_ns: u64,
    /// Absolute simulation time in nanoseconds.
    pub mujoco_time_ns: u64,
}

impl ClockAdvanceReq {
    /// Pack into a byte array (little-endian).
    pub fn pack(&self) -> [u8; 16] {
        let mut b = [0u8; 16];
        b[0..8].copy_from_slice(&self.delta_ns.to_le_bytes());
        b[8..16].copy_from_slice(&self.mujoco_time_ns.to_le_bytes());
        b
    }

    /// Unpack from a byte array (little-endian).
    pub fn unpack(b: &[u8; 16]) -> Self {
        Self {
            delta_ns: u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]),
            mujoco_time_ns: u64::from_le_bytes([
                b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15],
            ]),
        }
    }

    /// Unpack from a byte slice. Returns None if the slice is not the correct length.
    pub fn unpack_slice(b: &[u8]) -> Option<Self> {
        b.get(0..core::mem::size_of::<Self>())?.try_into().ok().map(Self::unpack)
    }
}

/// Clock ready response sent from node to TimeAuthority.
#[repr(C, packed)]
#[derive(Debug, Copy, Clone, Default)]
pub struct ClockReadyResp {
    /// Current virtual time in nanoseconds.
    pub current_vtime_ns: u64,
    /// Number of frames processed in this quantum.
    pub n_frames: u32,
    /// Error code (0=OK, 1=STALL).
    pub error_code: u32,
}

impl ClockReadyResp {
    /// Pack into a byte array (little-endian).
    pub fn pack(&self) -> [u8; 16] {
        let mut b = [0u8; 16];
        b[0..8].copy_from_slice(&self.current_vtime_ns.to_le_bytes());
        b[8..12].copy_from_slice(&self.n_frames.to_le_bytes());
        b[12..16].copy_from_slice(&self.error_code.to_le_bytes());
        b
    }

    /// Unpack from a byte array (little-endian).
    pub fn unpack(b: &[u8; 16]) -> Self {
        Self {
            current_vtime_ns: u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]),
            n_frames: u32::from_le_bytes([b[8], b[9], b[10], b[11]]),
            error_code: u32::from_le_bytes([b[12], b[13], b[14], b[15]]),
        }
    }

    /// Unpack from a byte slice. Returns None if the slice is not the correct length.
    pub fn unpack_slice(b: &[u8]) -> Option<Self> {
        b.get(0..core::mem::size_of::<Self>())?.try_into().ok().map(Self::unpack)
    }
}

/// Header prepended to every Zenoh message for deterministic delivery.
#[repr(C, packed)]
#[derive(Debug, Copy, Clone, Default)]
pub struct ZenohFrameHeader {
    /// Virtual time (nanoseconds) when this frame should be delivered.
    pub delivery_vtime_ns: u64,
    /// Size of the payload following this header.
    pub size: u32,
}

impl ZenohFrameHeader {
    /// Pack into a byte array (little-endian).
    pub fn pack(&self) -> [u8; 12] {
        let mut b = [0u8; 12];
        b[0..8].copy_from_slice(&self.delivery_vtime_ns.to_le_bytes());
        b[8..12].copy_from_slice(&self.size.to_le_bytes());
        b
    }

    /// Unpack from a byte array (little-endian).
    pub fn unpack(b: &[u8; 12]) -> Self {
        Self {
            delivery_vtime_ns: u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]),
            size: u32::from_le_bytes([b[8], b[9], b[10], b[11]]),
        }
    }

    /// Unpack from a byte slice. Returns None if the slice is not the correct length.
    pub fn unpack_slice(b: &[u8]) -> Option<Self> {
        b.get(0..core::mem::size_of::<Self>())?.try_into().ok().map(Self::unpack)
    }
}

#[repr(C, packed)]
#[derive(Debug, Copy, Clone, Default)]
/// A struct
pub struct ZenohSPIHeader {
    /// A struct field
    pub delivery_vtime_ns: u64,
    /// A struct field
    pub size: u32,
    /// A struct field
    pub cs: bool,
    /// A struct field
    pub cs_index: u8,
    /// A struct field
    pub _padding: [u8; 2],
}

impl ZenohSPIHeader {
    /// Pack into a byte array (little-endian).
    pub fn pack(&self) -> [u8; 16] {
        let mut b = [0u8; 16];
        b[0..8].copy_from_slice(&self.delivery_vtime_ns.to_le_bytes());
        b[8..12].copy_from_slice(&self.size.to_le_bytes());
        b[12] = if self.cs { 1 } else { 0 };
        b[13] = self.cs_index;
        // 14 and 15 are padding
        b
    }

    /// Unpack from a byte array (little-endian).
    pub fn unpack(b: &[u8; 16]) -> Self {
        Self {
            delivery_vtime_ns: u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]),
            size: u32::from_le_bytes([b[8], b[9], b[10], b[11]]),
            cs: b[12] != 0,
            cs_index: b[13],
            _padding: [b[14], b[15]],
        }
    }

    /// Unpack from a byte slice. Returns None if the slice is not the correct length.
    pub fn unpack_slice(b: &[u8]) -> Option<Self> {
        b.get(0..core::mem::size_of::<Self>())?.try_into().ok().map(Self::unpack)
    }
}

// Both Rust (zenoh-chardev) and Python (uart_stress_test.py) assume this is
// exactly 12 bytes with no padding.  Enforce it at compile time.
const _: () = assert!(
    core::mem::size_of::<ZenohFrameHeader>() == 12,
    "ZenohFrameHeader must be exactly 12 bytes (u64 + u32, packed)"
);

const _: () = assert!(
    core::mem::size_of::<ZenohSPIHeader>() == 16,
    "ZenohSPIHeader must be exactly 16 bytes"
);

// Minimal manual generation of FlatBuffer bindings for TraceEvent
#[allow(dead_code, non_snake_case)]
/// A module
pub mod telemetry_fb {
    use flatbuffers::{FlatBufferBuilder, WIPOffset};

    #[derive(Copy, Clone, PartialEq, Debug)]
    #[repr(i8)]
    /// An enum
    pub enum TraceEventType {
        /// A variant
        CpuState = 0,
        /// A variant
        Irq = 1,
        /// A variant
        Peripheral = 2,
    }

    /// A struct
    pub struct TraceEventArgs<'a> {
        /// A struct field
        pub timestamp_ns: u64,
        /// A struct field
        pub type_: TraceEventType,
        /// A struct field
        pub id: u32,
        /// A struct field
        pub value: u32,
        /// A struct field
        pub device_name: Option<WIPOffset<&'a str>>,
    }

    /// A function
    pub fn create_trace_event<'a>(
        fbb: &mut FlatBufferBuilder<'a>,
        args: &TraceEventArgs<'a>,
    ) -> WIPOffset<flatbuffers::Table<'a>> {
        let start = fbb.start_table();
        fbb.push_slot(4, args.timestamp_ns, 0);
        fbb.push_slot(8, args.id, 0);
        fbb.push_slot(10, args.value, 0);
        if let Some(x) = args.device_name {
            fbb.push_slot_always(12, x);
        }
        fbb.push_slot(6, args.type_ as i8, 0);
        let end = fbb.end_table(start);
        WIPOffset::new(end.value())
    }
}

/// A struct
pub struct TraceEvent {
    /// A struct field
    pub timestamp_ns: u64,
    /// A struct field
    pub event_type: i8,
    /// A struct field
    pub id: u32,
    /// A struct field
    pub value: u32,
    /// A struct field
    pub device_name: Option<String>,
}

/// Error codes returned in `ClockReadyResp.error_code`.
pub const CLOCK_ERROR_OK: u32 = 0;
/// A constant
pub const CLOCK_ERROR_STALL: u32 = 1;
/// A constant
pub const CLOCK_ERROR_ZENOH: u32 = 2;

/// Minimum payload size for a `ClockAdvanceReq` message.
pub const CLOCK_ADVANCE_REQ_SIZE: usize = core::mem::size_of::<ClockAdvanceReq>();
/// Exact byte size for a `ClockReadyResp` message.
pub const CLOCK_READY_RESP_SIZE: usize = core::mem::size_of::<ClockReadyResp>();
/// Exact byte size for a `ZenohFrameHeader`.
pub const ZENOH_FRAME_HEADER_SIZE: usize = core::mem::size_of::<ZenohFrameHeader>();

const _: () = assert!(
    core::mem::size_of::<VirtmcuHandshake>() == 8,
    "VirtmcuHandshake must be exactly 8 bytes"
);
const _: () = assert!(
    core::mem::size_of::<MmioReq>() == 32,
    "MmioReq must be exactly 32 bytes (1+1+2+4+8+8+8)"
);
const _: () =
    assert!(core::mem::size_of::<SyscMsg>() == 16, "SyscMsg must be exactly 16 bytes (4+4+8)");
const _: () = assert!(
    core::mem::size_of::<ClockAdvanceReq>() == 16,
    "ClockAdvanceReq must be exactly 16 bytes (8+8)"
);
const _: () = assert!(
    core::mem::size_of::<ClockReadyResp>() == 16,
    "ClockReadyResp must be exactly 16 bytes (8+4+4)"
);

/// Encode a `ZenohFrameHeader` + payload into a byte vector (little-endian).
pub fn encode_frame(delivery_vtime_ns: u64, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(ZENOH_FRAME_HEADER_SIZE + payload.len());
    let mut header_bytes = [0u8; 12];
    header_bytes[0..8].copy_from_slice(&delivery_vtime_ns.to_le_bytes());
    header_bytes[8..12].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&header_bytes);
    out.extend_from_slice(payload);
    out
}

/// Decode a `ZenohFrameHeader` from the first 12 bytes of `data`.
///
/// Returns `None` if `data` is shorter than `ZENOH_FRAME_HEADER_SIZE`.
pub fn decode_frame(data: &[u8]) -> Option<(ZenohFrameHeader, &[u8])> {
    let header = ZenohFrameHeader::unpack_slice(data)?;
    Some((header, &data[ZENOH_FRAME_HEADER_SIZE..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Delivery queue ordering (mirrors zenoh-chardev/zenoh-netdev OrderedPacket)
    // zenoh-chardev uses BinaryHeap<OrderedPacket> as a min-heap by vtime.
    // The Ord impl inverts comparison so the heap pops the lowest vtime first.
    // These tests validate the invariant without needing QEMU FFI.

    use std::cmp::Ordering as CmpOrd;
    use std::collections::BinaryHeap;

    #[derive(Debug, Eq, PartialEq)]
    struct TestPacket {
        vtime: u64,
    }
    impl Ord for TestPacket {
        fn cmp(&self, other: &Self) -> CmpOrd {
            other.vtime.cmp(&self.vtime) // inverted for min-heap
        }
    }
    impl PartialOrd for TestPacket {
        fn partial_cmp(&self, other: &Self) -> Option<CmpOrd> {
            Some(self.cmp(other))
        }
    }

    #[test]
    fn test_delivery_queue_min_heap_ordering() -> Result<(), String> {
        let mut heap = BinaryHeap::new();
        heap.push(TestPacket { vtime: 3_000 });
        heap.push(TestPacket { vtime: 1_000 });
        heap.push(TestPacket { vtime: 2_000 });
        assert_eq!(heap.pop().ok_or("Empty")?.vtime, 1_000);
        assert_eq!(heap.pop().ok_or("Empty")?.vtime, 2_000);
        assert_eq!(heap.pop().ok_or("Empty")?.vtime, 3_000);
        Ok(())
    }

    #[test]
    fn test_delivery_queue_vtime_zero_first() -> Result<(), String> {
        let mut heap = BinaryHeap::new();
        heap.push(TestPacket { vtime: 1_000_000 });
        heap.push(TestPacket { vtime: 0 });
        assert_eq!(heap.pop().ok_or("Empty")?.vtime, 0);
        Ok(())
    }

    #[test]
    fn test_delivery_queue_vtime_max_last() -> Result<(), String> {
        let mut heap = BinaryHeap::new();
        heap.push(TestPacket { vtime: u64::MAX });
        heap.push(TestPacket { vtime: 1 });
        assert_eq!(heap.pop().ok_or("Empty")?.vtime, 1);
        assert_eq!(heap.pop().ok_or("Empty")?.vtime, u64::MAX);
        Ok(())
    }

    #[test]
    fn test_delivery_queue_equal_vtimes_both_dequeued() -> Result<(), String> {
        let mut heap = BinaryHeap::new();
        heap.push(TestPacket { vtime: 500 });
        heap.push(TestPacket { vtime: 500 });
        assert_eq!(heap.len(), 2);
        heap.pop().ok_or("Empty")?;
        heap.pop().ok_or("Empty")?;
        assert!(heap.is_empty());
        Ok(())
    }

    #[test]
    fn test_delivery_queue_large_sequence_monotonic() -> Result<(), String> {
        const N: usize = 10_000;
        let mut heap = BinaryHeap::new();
        for i in (0..N).rev() {
            heap.push(TestPacket { vtime: i as u64 });
        }
        let mut prev = 0u64;
        for _ in 0..N {
            let p = heap.pop().ok_or("Empty")?;
            assert!(p.vtime >= prev, "out-of-order: {} < {}", p.vtime, prev);
            prev = p.vtime;
        }
        Ok(())
    }

    #[test]
    fn test_delivery_queue_inverted_cmp() {
        let a = TestPacket { vtime: 1 };
        let b = TestPacket { vtime: 2 };
        assert_eq!(a.cmp(&b), CmpOrd::Greater); // lower vtime → "greater" priority
        assert_eq!(b.cmp(&a), CmpOrd::Less);
    }

    // ── Zenoh topic naming conventions ────────────────────────────────────────

    #[test]
    fn test_chardev_rx_topic() {
        let base = "sim/chardev";
        assert_eq!(format!("{base}/0/rx"), "sim/chardev/0/rx");
        assert_eq!(format!("{base}/1/rx"), "sim/chardev/1/rx");
    }

    #[test]
    fn test_chardev_tx_topic() {
        let base = "sim/chardev";
        assert_eq!(format!("{base}/0/tx"), "sim/chardev/0/tx");
    }

    #[test]
    fn test_chardev_rx_tx_topics_distinct() {
        let base = "sim/chardev";
        let rx = format!("{base}/0/rx");
        let tx = format!("{base}/0/tx");
        assert_ne!(rx, tx);
    }

    #[test]
    fn test_clock_topic_format() {
        assert_eq!(format!("sim/clock/advance/{}", 0), "sim/clock/advance/0");
        assert_eq!(format!("sim/clock/advance/{}", 3), "sim/clock/advance/3");
    }

    #[test]
    fn test_multi_node_chardev_isolation() {
        let base = "sim/chardev";
        let rx0 = format!("{base}/0/rx");
        let rx1 = format!("{base}/1/rx");
        assert_ne!(rx0, rx1, "node 0 and node 1 must use different topics");
    }

    // ── Struct size assertions ────────────────────────────────────────────────

    #[test]
    fn test_virtmcu_handshake_size() {
        assert_eq!(core::mem::size_of::<VirtmcuHandshake>(), 8);
    }

    #[test]
    fn test_mmio_req_size() {
        assert_eq!(core::mem::size_of::<MmioReq>(), 32);
    }

    #[test]
    fn test_sysc_msg_size() {
        assert_eq!(core::mem::size_of::<SyscMsg>(), 16);
    }

    #[test]
    fn test_clock_advance_req_size() {
        assert_eq!(core::mem::size_of::<ClockAdvanceReq>(), 16);
    }

    #[test]
    fn test_clock_ready_resp_size() {
        assert_eq!(core::mem::size_of::<ClockReadyResp>(), 16);
    }

    #[test]
    fn test_zenoh_frame_header_size() {
        assert_eq!(core::mem::size_of::<ZenohFrameHeader>(), 12);
        assert_eq!(ZENOH_FRAME_HEADER_SIZE, 12);
    }

    // ── Wire format: ZenohFrameHeader ────────────────────────────────────────

    #[test]
    fn test_encode_decode_round_trip() -> Result<(), String> {
        let payload = b"hello";
        let frame = encode_frame(12345678, payload);
        assert_eq!(frame.len(), 12 + 5);

        let (hdr, rest) = decode_frame(&frame).ok_or("Decode failed")?;
        assert_eq!({ hdr.delivery_vtime_ns }, 12345678u64);
        assert_eq!({ hdr.size }, 5u32);
        assert_eq!(rest, payload);
        Ok(())
    }

    #[test]
    fn test_encode_empty_payload() -> Result<(), String> {
        let frame = encode_frame(0, b"");
        let (hdr, rest) = decode_frame(&frame).ok_or("Decode failed")?;
        assert_eq!({ hdr.delivery_vtime_ns }, 0u64);
        assert_eq!({ hdr.size }, 0u32);
        assert_eq!(rest, b"");
        Ok(())
    }

    #[test]
    fn test_encode_vtime_zero() -> Result<(), String> {
        let frame = encode_frame(0, b"X");
        let (hdr, _) = decode_frame(&frame).ok_or("Decode failed")?;
        assert_eq!({ hdr.delivery_vtime_ns }, 0u64);
        Ok(())
    }

    #[test]
    fn test_encode_vtime_max_u64() -> Result<(), String> {
        let max = u64::MAX;
        let frame = encode_frame(max, b"X");
        let (hdr, _) = decode_frame(&frame).ok_or("Decode failed")?;
        assert_eq!({ hdr.delivery_vtime_ns }, max);
        Ok(())
    }

    #[test]
    fn test_decode_rejects_short_data() {
        assert!(decode_frame(&[]).is_none());
        assert!(decode_frame(&[0u8; 11]).is_none());
    }

    #[test]
    fn test_decode_accepts_exact_header() {
        let frame = encode_frame(1, b"");
        assert!(decode_frame(&frame).is_some());
    }

    #[test]
    fn test_little_endian_vtime() {
        // 0x0102030405060708 in LE = bytes [08, 07, 06, 05, 04, 03, 02, 01]
        let vtime: u64 = 0x0102030405060708;
        let frame = encode_frame(vtime, b"");
        assert_eq!(&frame[0..8], &[0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01]);
    }

    #[test]
    fn test_little_endian_size() {
        // size = 0x00000005 in LE = bytes [05, 00, 00, 00]
        let frame = encode_frame(0, b"hello");
        assert_eq!(&frame[8..12], &[0x05, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn test_vtime_ordering() -> Result<(), String> {
        let earlier = encode_frame(1_000_000, b"A");
        let later = encode_frame(2_000_000, b"A");
        let (h1, _) = decode_frame(&earlier).ok_or("Decode failed")?;
        let (h2, _) = decode_frame(&later).ok_or("Decode failed")?;
        assert!({ h1.delivery_vtime_ns } < { h2.delivery_vtime_ns });
        Ok(())
    }

    #[test]
    fn test_10mbps_baud_interval_ns() {
        // 10 Mbps = 1_250_000 bytes/s → 800 ns/byte
        const BAUD_10MBPS_NS: u64 = 1_000_000_000 / 1_250_000;
        assert_eq!(BAUD_10MBPS_NS, 800);
    }

    #[test]
    fn test_encode_decode_sequence_monotonic() -> Result<(), String> {
        const N: u64 = 1_000;
        const START: u64 = 10_000_000;
        const STEP: u64 = 800;
        for i in 0..N {
            let vtime = START + i * STEP;
            let frame = encode_frame(vtime, b"X");
            let (hdr, payload) = decode_frame(&frame).ok_or("Decode failed")?;
            assert_eq!({ hdr.delivery_vtime_ns }, vtime, "frame {i} vtime mismatch");
            assert_eq!({ hdr.size }, 1u32);
            assert_eq!(payload, b"X");
        }
        Ok(())
    }

    // ── Wire format: ClockAdvanceReq ─────────────────────────────────────────
    // NOTE: repr(C, packed) fields must be copied out before comparison to
    // avoid creating misaligned references (Rust E0793).  Use `{ s.field }`.

    #[test]
    fn test_clock_advance_req_round_trip() {
        let req = ClockAdvanceReq { delta_ns: 10_000_000, mujoco_time_ns: 42 };
        let bytes = req.pack();
        let req2 = ClockAdvanceReq::unpack(&bytes);
        assert_eq!({ req.delta_ns }, { req2.delta_ns });
        assert_eq!({ req.mujoco_time_ns }, { req2.mujoco_time_ns });
    }

    #[test]
    fn test_clock_advance_req_le_encoding() {
        let req = ClockAdvanceReq { delta_ns: 0x0102030405060708, mujoco_time_ns: 0 };
        let bytes = req.pack();
        assert_eq!(&bytes[0..8], &[0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01]);
    }

    #[test]
    fn test_clock_advance_req_zero() {
        let req = ClockAdvanceReq { delta_ns: 0, mujoco_time_ns: 0 };
        let bytes = req.pack();
        assert_eq!(bytes, [0u8; 16]);
    }

    // ── Wire format: ClockReadyResp ───────────────────────────────────────────

    #[test]
    fn test_clock_ready_resp_ok() {
        let resp = ClockReadyResp {
            current_vtime_ns: 10_000_000,
            n_frames: 50,
            error_code: CLOCK_ERROR_OK,
        };
        let bytes = resp.pack();
        let resp2 = ClockReadyResp::unpack(&bytes);
        assert_eq!({ resp2.current_vtime_ns }, 10_000_000u64);
        assert_eq!({ resp2.n_frames }, 50u32);
        assert_eq!({ resp2.error_code }, CLOCK_ERROR_OK);
    }

    #[test]
    fn test_clock_ready_resp_stall() {
        let resp =
            ClockReadyResp { current_vtime_ns: 0, n_frames: 0, error_code: CLOCK_ERROR_STALL };
        let bytes = resp.pack();
        let resp2 = ClockReadyResp::unpack(&bytes);
        assert_eq!({ resp2.error_code }, CLOCK_ERROR_STALL);
    }

    #[test]
    fn test_clock_error_codes_distinct() {
        assert_ne!(CLOCK_ERROR_OK, CLOCK_ERROR_STALL);
        assert_ne!(CLOCK_ERROR_OK, CLOCK_ERROR_ZENOH);
        assert_ne!(CLOCK_ERROR_STALL, CLOCK_ERROR_ZENOH);
    }

    // ── Wire format: MmioReq ─────────────────────────────────────────────────

    #[test]
    fn test_mmio_req_read_type() {
        let req = MmioReq { type_: MMIO_REQ_READ, ..Default::default() };
        assert_eq!({ req.type_ }, 0u8);
    }

    #[test]
    fn test_mmio_req_write_type() {
        let req = MmioReq { type_: MMIO_REQ_WRITE, ..Default::default() };
        assert_eq!({ req.type_ }, 1u8);
    }

    #[test]
    fn test_mmio_req_cross_language_pack() {
        let req = MmioReq {
            type_: 1,
            size: 4,
            reserved1: 0x1234,
            reserved2: 0x56789ABC,
            vtime_ns: 0x1122334455667788,
            addr: 0x99AABBCCDDEEFF00,
            data: 0x1020304050607080,
        };
        let bytes = req.pack();

        let expected: [u8; 32] = [
            0x01, 0x04, 0x34, 0x12, 0xBC, 0x9A, 0x78, 0x56, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33,
            0x22, 0x11, 0x00, 0xFF, 0xEE, 0xDD, 0xCC, 0xBB, 0xAA, 0x99, 0x80, 0x70, 0x60, 0x50,
            0x40, 0x30, 0x20, 0x10,
        ];

        assert_eq!(
            bytes, expected,
            "Rust pack() output must exactly match Python struct.pack('<BBHIQQQ')"
        );
    }

    #[test]
    fn test_mmio_req_round_trip() {
        let req = MmioReq {
            type_: MMIO_REQ_WRITE,
            size: 4,
            reserved1: 0,
            reserved2: 0,
            vtime_ns: 999_999,
            addr: 0x1000_0000,
            data: 0xDEAD_BEEF,
        };
        let bytes = req.pack();
        let req2 = MmioReq::unpack(&bytes);
        assert_eq!({ req2.type_ }, MMIO_REQ_WRITE);
        assert_eq!({ req2.size }, 4u8);
        assert_eq!({ req2.vtime_ns }, 999_999u64);
        assert_eq!({ req2.addr }, 0x1000_0000u64);
        assert_eq!({ req2.data }, 0xDEAD_BEEFu64);
    }

    // ── Wire format: SyscMsg ─────────────────────────────────────────────────

    #[test]
    fn test_sysc_msg_types_distinct() {
        assert_ne!(SYSC_MSG_RESP, SYSC_MSG_IRQ_SET);
        assert_ne!(SYSC_MSG_RESP, SYSC_MSG_IRQ_CLEAR);
        assert_ne!(SYSC_MSG_IRQ_SET, SYSC_MSG_IRQ_CLEAR);
    }

    #[test]
    fn test_sysc_msg_irq_round_trip() {
        let msg = SyscMsg { type_: SYSC_MSG_IRQ_SET, irq_num: 7, data: 1 };
        let bytes = msg.pack();
        let msg2 = SyscMsg::unpack(&bytes);
        assert_eq!({ msg2.type_ }, SYSC_MSG_IRQ_SET);
        assert_eq!({ msg2.irq_num }, 7u32);
        assert_eq!({ msg2.data }, 1u64);
    }

    // ── Proto magic / version ─────────────────────────────────────────────────

    #[test]
    fn test_proto_magic_value() {
        // VIRTMCU_PROTO_MAGIC = 0x564D4355
        assert_eq!(VIRTMCU_PROTO_MAGIC, 0x564D_4355);
        // In little-endian bytes on wire: [0x55, 0x43, 0x4D, 0x56] = "UCMV"
        let bytes = VIRTMCU_PROTO_MAGIC.to_le_bytes();
        assert_eq!(bytes, [0x55, 0x43, 0x4D, 0x56]);
    }

    #[test]
    fn test_proto_version_is_one() {
        assert_eq!(VIRTMCU_PROTO_VERSION, 1);
    }

    #[test]
    fn test_handshake_round_trip() {
        let hs = VirtmcuHandshake { magic: VIRTMCU_PROTO_MAGIC, version: VIRTMCU_PROTO_VERSION };
        let bytes = hs.pack();
        let hs2 = VirtmcuHandshake::unpack(&bytes);
        assert_eq!({ hs2.magic }, VIRTMCU_PROTO_MAGIC);
        assert_eq!({ hs2.version }, VIRTMCU_PROTO_VERSION);
    }
}
