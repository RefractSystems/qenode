// std is required: flatbuffers dependency requires std
#![deny(missing_docs)]
#![doc = "The crate"]

extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

#[allow(
    clippy::all,
    missing_docs,
    clippy::unwrap_used,
    clippy::missing_safety_doc,
    clippy::undocumented_unsafe_blocks,
    clippy::extra_unused_lifetimes
)]
pub mod can_generated;
#[allow(
    clippy::all,
    missing_docs,
    clippy::unwrap_used,
    clippy::missing_safety_doc,
    clippy::undocumented_unsafe_blocks,
    clippy::extra_unused_lifetimes
)]
pub mod core_generated;
#[allow(
    clippy::all,
    missing_docs,
    clippy::unwrap_used,
    clippy::missing_safety_doc,
    clippy::undocumented_unsafe_blocks,
    clippy::extra_unused_lifetimes
)]
pub mod flexray_generated;
#[allow(
    clippy::all,
    missing_docs,
    clippy::unwrap_used,
    clippy::missing_safety_doc,
    clippy::undocumented_unsafe_blocks,
    clippy::extra_unused_lifetimes
)]
pub mod lin_generated;
#[allow(
    clippy::all,
    missing_docs,
    clippy::unwrap_used,
    clippy::missing_safety_doc,
    clippy::undocumented_unsafe_blocks,
    clippy::extra_unused_lifetimes
)]
pub mod rf_generated;
#[allow(
    clippy::all,
    missing_docs,
    clippy::unwrap_used,
    clippy::missing_safety_doc,
    clippy::undocumented_unsafe_blocks,
    clippy::extra_unused_lifetimes
)]
pub mod telemetry_generated;
#[allow(
    clippy::all,
    missing_docs,
    clippy::unwrap_used,
    clippy::missing_safety_doc,
    clippy::undocumented_unsafe_blocks,
    clippy::extra_unused_lifetimes
)]
pub mod wifi_generated;
pub use core_generated::virtmcu::core::*;

/// Extension trait for FlatBuffer structs
pub trait FlatBufferStructExt: Sized {
    /// Unpack slice
    fn unpack_slice(b: &[u8]) -> Option<Self>;
    /// Pack
    fn pack(&self) -> &[u8];
}
impl FlatBufferStructExt for VirtmcuHandshake {
    fn unpack_slice(b: &[u8]) -> Option<Self> {
        b.get(0..8)?.try_into().ok().map(Self)
    }
    fn pack(&self) -> &[u8] {
        &self.0
    }
}
impl FlatBufferStructExt for MmioReq {
    fn unpack_slice(b: &[u8]) -> Option<Self> {
        b.get(0..32)?.try_into().ok().map(Self)
    }
    fn pack(&self) -> &[u8] {
        &self.0
    }
}
impl FlatBufferStructExt for SyscMsg {
    fn unpack_slice(b: &[u8]) -> Option<Self> {
        b.get(0..16)?.try_into().ok().map(Self)
    }
    fn pack(&self) -> &[u8] {
        &self.0
    }
}
impl FlatBufferStructExt for ClockAdvanceReq {
    fn unpack_slice(b: &[u8]) -> Option<Self> {
        b.get(0..24)?.try_into().ok().map(Self)
    }
    fn pack(&self) -> &[u8] {
        &self.0
    }
}
impl FlatBufferStructExt for ClockReadyResp {
    fn unpack_slice(b: &[u8]) -> Option<Self> {
        b.get(0..24)?.try_into().ok().map(Self)
    }
    fn pack(&self) -> &[u8] {
        &self.0
    }
}
impl FlatBufferStructExt for ZenohFrameHeader {
    fn unpack_slice(b: &[u8]) -> Option<Self> {
        b.get(0..24)?.try_into().ok().map(Self)
    }
    fn pack(&self) -> &[u8] {
        &self.0
    }
}
impl FlatBufferStructExt for ZenohSPIHeader {
    fn unpack_slice(b: &[u8]) -> Option<Self> {
        b.get(0..24)?.try_into().ok().map(Self)
    }
    fn pack(&self) -> &[u8] {
        &self.0
    }
}

/// A constant
pub const VIRTMCU_PROTO_MAGIC: u32 = 0x564D4355;
/// A constant
pub const VIRTMCU_PROTO_VERSION: u32 = 1;

/// A constant
pub const MMIO_REQ_READ: u8 = 0;
/// A constant
pub const MMIO_REQ_WRITE: u8 = 1;

/// A constant
pub const SYSC_MSG_RESP: u32 = 0;
/// A constant
pub const SYSC_MSG_IRQ_SET: u32 = 1;
/// A constant
pub const SYSC_MSG_IRQ_CLEAR: u32 = 2;

/// Abstract transport for clock synchronization between TimeAuthority and node.
pub trait ClockSyncTransport: Send + Sync {
    /// Blocks until a clock advancement request is received, a timeout occurs, or transport is closed.
    /// Returns the request and a responder trait object.
    fn recv_advance(
        &self,
        timeout: core::time::Duration,
    ) -> Option<(ClockAdvanceReq, Box<dyn ClockSyncResponder>)>;

    /// Sends a virtual time heartbeat signal to external listeners.
    fn send_vtime_heartbeat(&self, _vtime_ns: u64) {}

    /// Closes the transport, unblocking any pending `recv_advance` calls.
    fn close(&self) {}
}

/// Abstract responder for a specific clock advancement request.
pub trait ClockSyncResponder: Send + Sync {
    /// Sends a clock ready response back to the TimeAuthority.
    fn send_ready(&self, resp: ClockReadyResp) -> Result<(), alloc::string::String>;
}

/// Unix socket-based clock synchronization transport.
#[cfg(feature = "std")]
pub struct UnixSocketClockTransport {
    path: std::path::PathBuf,
    stream: std::sync::Mutex<Option<std::os::unix::net::UnixStream>>,
}

#[cfg(feature = "std")]
impl UnixSocketClockTransport {
    /// Creates a new `UnixSocketClockTransport` that will connect to the given path.
    pub fn new<P: AsRef<std::path::Path>>(path: P) -> Self {
        Self { path: path.as_ref().to_path_buf(), stream: std::sync::Mutex::new(None) }
    }

    /// Connects to the socket.
    pub fn connect(&self) -> std::io::Result<()> {
        let stream = std::os::unix::net::UnixStream::connect(&self.path)?;
        let mut guard = self
            .stream
            .lock()
            .map_err(|e| std::io::Error::other(format!("Mutex poisoned: {e}")))?;
        *guard = Some(stream);
        Ok(())
    }
}

#[cfg(feature = "std")]
impl ClockSyncTransport for UnixSocketClockTransport {
    fn recv_advance(
        &self,
        timeout: core::time::Duration,
    ) -> Option<(ClockAdvanceReq, Box<dyn ClockSyncResponder>)> {
        use std::io::Read;
        let mut buf = [0u8; 24];

        let mut stream_guard = self.stream.lock().ok()?;
        if stream_guard.is_none() {
            drop(stream_guard);
            self.connect().ok()?;
            stream_guard = self.stream.lock().ok()?;
        }

        let stream = stream_guard.as_mut()?;
        let _ = stream.set_read_timeout(Some(timeout));
        if stream.read_exact(&mut buf).is_err() {
            return None;
        }

        let req = ClockAdvanceReq::unpack_slice(&buf)?;
        let responder: Box<dyn ClockSyncResponder> =
            Box::new(UnixSocketResponder { stream: stream.try_clone().ok()? });
        Some((req, responder))
    }

    fn close(&self) {
        if let Ok(mut guard) = self.stream.lock() {
            if let Some(stream) = guard.as_mut() {
                let _ = stream.shutdown(std::net::Shutdown::Both);
            }
        }
    }
}

/// Macro to explicitly export a function across the FFI boundary.
/// This enforces `#[no_mangle]` to ensure the dynamic linker can find the symbol by name.
#[macro_export]
macro_rules! virtmcu_export {
    (
        $(#[$meta:meta])*
        $vis:vis extern "C" fn $name:ident($($arg:ident: $arg_ty:ty),* $(,)?) $(-> $ret:ty)? $body:block
    ) => {
        $(#[$meta])*
        #[no_mangle]
        $vis extern "C" fn $name($($arg: $arg_ty),*) $(-> $ret)? $body
    };
}

#[cfg(feature = "std")]
struct UnixSocketResponder {
    stream: std::os::unix::net::UnixStream,
}

#[cfg(feature = "std")]
impl ClockSyncResponder for UnixSocketResponder {
    fn send_ready(&self, resp: ClockReadyResp) -> Result<(), alloc::string::String> {
        use std::io::Write;
        let mut stream = &self.stream;
        let bytes = resp.pack();
        stream.write_all(bytes).map_err(|e| format!("{e}"))?;
        stream.flush().map_err(|e| format!("{e}"))
    }
}

// Both Rust (chardev) and Python (uart_stress_test.py) assume this is
// exactly 20 bytes with no padding.  Enforce it at compile time.
const _: () = assert!(
    core::mem::size_of::<ZenohFrameHeader>() == 24,
    "ZenohFrameHeader must be exactly 24 bytes (u64 + u64 + u32, packed)"
);

const _: () = assert!(
    core::mem::size_of::<ZenohSPIHeader>() == 24,
    "ZenohSPIHeader must be exactly 24 bytes"
);

// Minimal manual generation of FlatBuffer bindings for TraceEvent
#[allow(dead_code, non_snake_case, clippy::undocumented_unsafe_blocks)]
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

/// Size of ZenohSPIHeader in bytes.
pub const ZENOH_SPI_HEADER_SIZE: usize = core::mem::size_of::<ZenohSPIHeader>();

/// Size of VirtmcuHandshake in bytes.
pub const VIRTMCU_HANDSHAKE_SIZE: usize = core::mem::size_of::<VirtmcuHandshake>();

/// Size of SyscMsg in bytes.
pub const SYSC_MSG_SIZE: usize = core::mem::size_of::<SyscMsg>();

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
    core::mem::size_of::<ClockAdvanceReq>() == 24,
    "ClockAdvanceReq must be exactly 24 bytes (8+8+8)"
);
const _: () = assert!(
    core::mem::size_of::<ClockReadyResp>() == 24,
    "ClockReadyResp must be exactly 24 bytes (8+4+4+8)"
);

/// Encode a `ZenohFrameHeader` + payload into a byte vector (little-endian).
/// Encode a `ZenohFrameHeader` + payload into a byte vector (little-endian).
pub fn encode_frame(delivery_vtime_ns: u64, sequence_number: u64, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(ZENOH_FRAME_HEADER_SIZE + payload.len());
    let header = ZenohFrameHeader::new(delivery_vtime_ns, sequence_number, payload.len() as u32);
    out.extend_from_slice(&header.0);
    out.extend_from_slice(payload);
    out
}

/// Decode a `ZenohFrameHeader` from the first 20 bytes of `data`.
///
/// Returns `None` if `data` is shorter than `ZENOH_FRAME_HEADER_SIZE`.
/// Decode a `ZenohFrameHeader` from the first 24 bytes of `data`.
pub fn decode_frame(data: &[u8]) -> Option<(ZenohFrameHeader, &[u8])> {
    if data.len() < ZENOH_FRAME_HEADER_SIZE {
        return None;
    }
    let mut buf = [0u8; 24];
    buf.copy_from_slice(&data[..ZENOH_FRAME_HEADER_SIZE]);
    let header = ZenohFrameHeader(buf);
    Some((header, &data[ZENOH_FRAME_HEADER_SIZE..]))
}

/// Callback type for data transport subscriptions.
pub type DataCallback = Box<dyn Fn(&[u8]) + Send + Sync>;

/// Abstract transport for emulated data plane (packets, signals).
///
/// This trait abstracts the underlying communication mechanism (e.g., Zenoh, Unix Sockets)
/// used for peripheral data traffic.
pub trait DataTransport: Send + Sync {
    /// Publishes a message to the emulated network on the given topic.
    fn publish(&self, topic: &str, payload: &[u8]) -> Result<(), String>;

    /// Subscribes to messages from the emulated network on the given topic.
    ///
    /// The provided callback will be invoked for each received message.
    fn subscribe(&self, topic: &str, callback: DataCallback) -> Result<(), String>;

    /// Performs a synchronous query (Request/Response) on the given topic.
    fn query(&self, _topic: &str, _payload: &[u8]) -> Result<Vec<u8>, String> {
        Err("Query not supported by this transport".to_owned())
    }

    /// Closes the transport.
    fn close(&self) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Delivery queue ordering (mirrors chardev/netdev OrderedPacket)
    // chardev uses BinaryHeap<OrderedPacket> as a min-heap by vtime + sequence.
    // The Ord impl inverts comparison so the heap pops the lowest vtime first.
    // These tests validate the invariant without needing QEMU FFI.

    use alloc::collections::BinaryHeap;
    use core::cmp::Ordering as CmpOrd;

    #[derive(Debug, Eq, PartialEq)]
    struct TestPacket {
        vtime: u64,
        sequence: u64,
    }

    impl Ord for TestPacket {
        fn cmp(&self, other: &Self) -> CmpOrd {
            // Invert the order to make BinaryHeap a min-heap
            other.vtime.cmp(&self.vtime).then_with(|| other.sequence.cmp(&self.sequence))
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
        heap.push(TestPacket { vtime: 3_000, sequence: 0 });
        heap.push(TestPacket { vtime: 1_000, sequence: 0 });
        heap.push(TestPacket { vtime: 2_000, sequence: 0 });
        assert_eq!(heap.pop().ok_or("Empty")?.vtime, 1_000);
        assert_eq!(heap.pop().ok_or("Empty")?.vtime, 2_000);
        assert_eq!(heap.pop().ok_or("Empty")?.vtime, 3_000);
        Ok(())
    }

    #[test]
    fn test_delivery_queue_sequence_ordering() -> Result<(), String> {
        let mut heap = BinaryHeap::new();
        heap.push(TestPacket { vtime: 1_000, sequence: 2 });
        heap.push(TestPacket { vtime: 1_000, sequence: 0 });
        heap.push(TestPacket { vtime: 1_000, sequence: 1 });
        assert_eq!(heap.pop().ok_or("Empty")?.sequence, 0);
        assert_eq!(heap.pop().ok_or("Empty")?.sequence, 1);
        assert_eq!(heap.pop().ok_or("Empty")?.sequence, 2);
        Ok(())
    }

    #[test]
    fn test_delivery_queue_vtime_zero_first() -> Result<(), String> {
        let mut heap = BinaryHeap::new();
        heap.push(TestPacket { vtime: 1_000_000, sequence: 0 });
        heap.push(TestPacket { vtime: 0, sequence: 0 });
        assert_eq!(heap.pop().ok_or("Empty")?.vtime, 0);
        Ok(())
    }

    #[test]
    fn test_delivery_queue_vtime_max_last() -> Result<(), String> {
        let mut heap = BinaryHeap::new();
        heap.push(TestPacket { vtime: u64::MAX, sequence: 0 });
        heap.push(TestPacket { vtime: 1, sequence: 0 });
        assert_eq!(heap.pop().ok_or("Empty")?.vtime, 1);
        assert_eq!(heap.pop().ok_or("Empty")?.vtime, u64::MAX);
        Ok(())
    }

    #[test]
    fn test_delivery_queue_equal_vtimes_both_dequeued() -> Result<(), String> {
        let mut heap = BinaryHeap::new();
        heap.push(TestPacket { vtime: 500, sequence: 0 });
        heap.push(TestPacket { vtime: 500, sequence: 1 });
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
            heap.push(TestPacket { vtime: i as u64, sequence: 0 });
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
        let a = TestPacket { vtime: 1, sequence: 0 };
        let b = TestPacket { vtime: 2, sequence: 0 };
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
        assert_eq!(core::mem::size_of::<ClockAdvanceReq>(), 24);
    }

    #[test]
    fn test_clock_ready_resp_size() {
        assert_eq!(core::mem::size_of::<ClockReadyResp>(), 24);
    }

    #[test]
    fn test_zenoh_frame_header_size() {
        assert_eq!(core::mem::size_of::<ZenohFrameHeader>(), 24);
        assert_eq!(ZENOH_FRAME_HEADER_SIZE, 24);
    }

    // ── Wire format: ZenohFrameHeader ────────────────────────────────────────

    #[test]
    fn test_encode_decode_round_trip() -> Result<(), String> {
        let payload = b"hello";
        let frame = encode_frame(12345678, 42, payload);
        assert_eq!(frame.len(), 24 + 5);

        let (hdr, rest) = decode_frame(&frame).ok_or("Decode failed")?;
        assert_eq!({ hdr.delivery_vtime_ns() }, 12345678u64);
        assert_eq!({ hdr.sequence_number() }, 42u64);
        assert_eq!({ hdr.size() }, 5u32);
        assert_eq!(rest, payload);
        Ok(())
    }

    #[test]
    fn test_encode_empty_payload() -> Result<(), String> {
        let frame = encode_frame(0, 0, b"");
        let (hdr, rest) = decode_frame(&frame).ok_or("Decode failed")?;
        assert_eq!({ hdr.delivery_vtime_ns() }, 0u64);
        assert_eq!({ hdr.sequence_number() }, 0u64);
        assert_eq!({ hdr.size() }, 0u32);
        assert_eq!(rest, b"");
        Ok(())
    }

    #[test]
    fn test_encode_vtime_zero() -> Result<(), String> {
        let frame = encode_frame(0, 0, b"X");
        let (hdr, _) = decode_frame(&frame).ok_or("Decode failed")?;
        assert_eq!({ hdr.delivery_vtime_ns() }, 0u64);
        Ok(())
    }

    #[test]
    fn test_encode_vtime_max_u64() -> Result<(), String> {
        let max = u64::MAX;
        let frame = encode_frame(max, 0, b"X");
        let (hdr, _) = decode_frame(&frame).ok_or("Decode failed")?;
        assert_eq!({ hdr.delivery_vtime_ns() }, max);
        Ok(())
    }

    #[test]
    fn test_decode_rejects_short_data() {
        assert!(decode_frame(&[]).is_none());
        assert!(decode_frame(&[0u8; 23]).is_none());
    }

    #[test]
    fn test_decode_accepts_exact_header() {
        let frame = encode_frame(1, 0, b"");
        assert!(decode_frame(&frame).is_some());
    }

    #[test]
    fn test_little_endian_vtime() {
        // 0x0102030405060708 in LE = bytes [08, 07, 06, 05, 04, 03, 02, 01]
        let vtime: u64 = 0x0102030405060708;
        let frame = encode_frame(vtime, 0, b"");
        assert_eq!(&frame[0..8], &[0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01]);
    }

    #[test]
    fn test_little_endian_sequence() {
        let seq: u64 = 0x0102030405060708;
        let frame = encode_frame(0, seq, b"");
        assert_eq!(&frame[8..16], &[0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01]);
    }

    #[test]
    fn test_little_endian_size() {
        // size = 0x00000005 in LE = bytes [05, 00, 00, 00]
        let frame = encode_frame(0, 0, b"hello");
        assert_eq!(&frame[16..20], &[0x05, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn test_vtime_ordering() -> Result<(), String> {
        let earlier = encode_frame(1_000_000, 0, b"A");
        let later = encode_frame(2_000_000, 0, b"A");
        let (h1, _) = decode_frame(&earlier).ok_or("Decode failed")?;
        let (h2, _) = decode_frame(&later).ok_or("Decode failed")?;
        assert!({ h1.delivery_vtime_ns() } < { h2.delivery_vtime_ns() });
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
            let frame = encode_frame(vtime, 0, b"X");
            let (hdr, payload) = decode_frame(&frame).ok_or("Decode failed")?;
            assert_eq!({ hdr.delivery_vtime_ns() }, vtime, "frame {i} vtime mismatch");
            assert_eq!({ hdr.size() }, 1u32);
            assert_eq!(payload, b"X");
        }
        Ok(())
    }

    // ── Wire format: ClockAdvanceReq ─────────────────────────────────────────
    // NOTE: repr(C, packed) fields must be copied out before comparison to
    // avoid creating misaligned references (Rust E0793).  Use `{ s.field }`.

    #[test]
    fn test_clock_advance_req_round_trip() {
        let req = ClockAdvanceReq::new(10_000_000, 42, 123);
        let bytes = req.pack();
        let req2 = ClockAdvanceReq::unpack_slice(bytes).unwrap();
        assert_eq!({ req.delta_ns() }, { req2.delta_ns() });
        assert_eq!({ req.mujoco_time_ns() }, { req2.mujoco_time_ns() });
        assert_eq!({ req.quantum_number() }, { req2.quantum_number() });
    }

    #[test]
    fn test_clock_advance_req_le_encoding() {
        let req = ClockAdvanceReq::new(0x0102030405060708, 0, 0x0A0B0C0D0E0F1011);
        let bytes = req.pack();
        assert_eq!(&bytes[0..8], &[0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01]);
        assert_eq!(&bytes[16..24], &[0x11, 0x10, 0x0F, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A]);
    }

    #[test]
    fn test_clock_advance_req_zero() {
        let req = ClockAdvanceReq::new(0, 0, 0);
        let bytes = req.pack();
        assert_eq!(bytes, [0u8; 24]);
    }

    // ── Wire format: ClockReadyResp ───────────────────────────────────────────

    #[test]
    fn test_clock_ready_resp_ok() {
        let resp = ClockReadyResp::new(10_000_000, 50, CLOCK_ERROR_OK, 99);
        let bytes = resp.pack();
        let resp2 = ClockReadyResp::unpack_slice(bytes).unwrap();
        assert_eq!({ resp2.current_vtime_ns() }, 10_000_000u64);
        assert_eq!({ resp2.n_frames() }, 50u32);
        assert_eq!({ resp2.error_code() }, CLOCK_ERROR_OK);
        assert_eq!({ resp2.quantum_number() }, 99u64);
    }

    #[test]
    fn test_clock_ready_resp_stall() {
        let resp = ClockReadyResp::new(0, 0, CLOCK_ERROR_STALL, 0);
        let bytes = resp.pack();
        let resp2 = ClockReadyResp::unpack_slice(bytes).unwrap();
        assert_eq!({ resp2.error_code() }, CLOCK_ERROR_STALL);
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
        let req = MmioReq::new(MMIO_REQ_READ, 0, 0, 0, 0, 0, 0);
        assert_eq!({ req.type_() }, 0u8);
    }

    #[test]
    fn test_mmio_req_write_type() {
        let req = MmioReq::new(MMIO_REQ_WRITE, 0, 0, 0, 0, 0, 0);
        assert_eq!({ req.type_() }, 1u8);
    }

    #[test]
    fn test_mmio_req_cross_language_pack() {
        let req = MmioReq::new(
            1,
            4,
            0x1234,
            0x56789ABC,
            0x1122334455667788,
            0x99AABBCCDDEEFF00,
            0x1020304050607080,
        );
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
        let req = MmioReq::new(MMIO_REQ_WRITE, 4, 0, 0, 999_999, 0x1000_0000, 0xDEAD_BEEF);
        let bytes = req.pack();
        let req2 = MmioReq::unpack_slice(bytes).unwrap();
        assert_eq!({ req2.type_() }, MMIO_REQ_WRITE);
        assert_eq!({ req2.size() }, 4u8);
        assert_eq!({ req2.vtime_ns() }, 999_999u64);
        assert_eq!({ req2.addr() }, 0x1000_0000u64);
        assert_eq!({ req2.data() }, 0xDEAD_BEEFu64);
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
        let msg = SyscMsg::new(SYSC_MSG_IRQ_SET, 7, 1);
        let bytes = msg.pack();
        let msg2 = SyscMsg::unpack_slice(bytes).unwrap();
        assert_eq!({ msg2.type_() }, SYSC_MSG_IRQ_SET);
        assert_eq!({ msg2.irq_num() }, 7u32);
        assert_eq!({ msg2.data() }, 1u64);
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
        let hs = VirtmcuHandshake::new(VIRTMCU_PROTO_MAGIC, VIRTMCU_PROTO_VERSION);
        let bytes = hs.pack();
        let hs2 = VirtmcuHandshake::unpack_slice(bytes).unwrap();
        assert_eq!({ hs2.magic() }, VIRTMCU_PROTO_MAGIC);
        assert_eq!({ hs2.version() }, VIRTMCU_PROTO_VERSION);
    }

    #[test]
    fn test_header_roundtrip() {
        let h = ZenohFrameHeader::new(12345, 7, 100);
        let bytes = h.pack();
        let h2 = ZenohFrameHeader::unpack_slice(bytes).unwrap();
        assert_eq!({ h.delivery_vtime_ns() }, { h2.delivery_vtime_ns() });
        assert_eq!({ h.sequence_number() }, { h2.sequence_number() });
        assert_eq!({ h.size() }, { h2.size() });
    }

    #[test]
    fn test_header_size_20() {
        assert_eq!(core::mem::size_of::<ZenohFrameHeader>(), 24);
        assert_eq!(ZENOH_FRAME_HEADER_SIZE, 24);
    }

    #[test]
    fn test_header_le_bytes() {
        let h = ZenohFrameHeader::new(1, 0, 0);
        let bytes = h.pack();
        assert_eq!(bytes[0], 0x01);
        assert_eq!(bytes[1], 0x00);
    }

    #[test]
    fn test_header_seq_ordering() {
        let mut frames = alloc::vec![
            ZenohFrameHeader::new(100, 4, 0),
            ZenohFrameHeader::new(100, 2, 0),
            ZenohFrameHeader::new(100, 0, 0),
            ZenohFrameHeader::new(100, 3, 0),
            ZenohFrameHeader::new(100, 1, 0),
        ];
        frames.sort_by_key(super::core_generated::virtmcu::core::ZenohFrameHeader::sequence_number);
        let seqs: alloc::vec::Vec<u64> = frames
            .iter()
            .map(super::core_generated::virtmcu::core::ZenohFrameHeader::sequence_number)
            .collect();
        assert_eq!(seqs, alloc::vec![0, 1, 2, 3, 4]);
    }
}
