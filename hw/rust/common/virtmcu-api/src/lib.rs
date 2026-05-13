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
// std is required: flatbuffers dependency requires std
#![deny(missing_docs)]
#![doc = "The crate"]

/// Topics module for standard Zenoh routing
pub mod topics;
extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

#[allow( // virtmcu-allow: allow reasoning="FlatBuffers-generated module"
    clippy::all, // virtmcu-allow: allow reasoning="FlatBuffers-generated module — machine-generated code, not hand-written"
    missing_docs,
    clippy::unwrap_used,
    clippy::missing_safety_doc,
    clippy::undocumented_unsafe_blocks,
    clippy::extra_unused_lifetimes
)]
pub mod can_generated;
#[allow( // virtmcu-allow: allow reasoning="FlatBuffers-generated module"
    clippy::all, // virtmcu-allow: allow reasoning="FlatBuffers-generated module — machine-generated code, not hand-written"
    missing_docs,
    clippy::unwrap_used,
    clippy::missing_safety_doc,
    clippy::undocumented_unsafe_blocks,
    clippy::extra_unused_lifetimes
)]
pub mod core_generated;
#[allow( // virtmcu-allow: allow reasoning="FlatBuffers-generated module"
    clippy::all, // virtmcu-allow: allow reasoning="FlatBuffers-generated module — machine-generated code, not hand-written"
    missing_docs,
    clippy::unwrap_used,
    clippy::missing_safety_doc,
    clippy::undocumented_unsafe_blocks,
    clippy::extra_unused_lifetimes
)]
pub mod flexray_generated;
#[allow( // virtmcu-allow: allow reasoning="FlatBuffers-generated module"
    clippy::all, // virtmcu-allow: allow reasoning="FlatBuffers-generated module — machine-generated code, not hand-written"
    missing_docs,
    clippy::unwrap_used,
    clippy::missing_safety_doc,
    clippy::undocumented_unsafe_blocks,
    clippy::extra_unused_lifetimes
)]
pub mod lin_generated;
pub use rf802154_generated::virtmcu::rf_802154 as rf802154;
#[allow( // virtmcu-allow: allow reasoning="FlatBuffers-generated module"
    clippy::all,
    missing_docs,
    clippy::unwrap_used,
    clippy::missing_safety_doc,
    clippy::undocumented_unsafe_blocks,
    clippy::extra_unused_lifetimes
)]
pub mod insn_trace_generated;
#[allow( // virtmcu-allow: allow reasoning="FlatBuffers-generated module"
    clippy::all,
    missing_docs,
    clippy::unwrap_used,
    clippy::missing_safety_doc,
    clippy::undocumented_unsafe_blocks,
    clippy::extra_unused_lifetimes
)]
pub mod physics_generated;
#[allow( // virtmcu-allow: allow reasoning="FlatBuffers-generated module"
    clippy::all, // virtmcu-allow: allow reasoning="FlatBuffers-generated module — machine-generated code, not hand-written"
    missing_docs,
    clippy::unwrap_used,
    clippy::missing_safety_doc,
    clippy::undocumented_unsafe_blocks,
    clippy::extra_unused_lifetimes
)]
pub mod rf802154_generated;
pub use insn_trace_generated::virtmcu::insn_trace as insn_trace_proto;
pub use physics_generated::virtmcu::physics as physics_proto;

/// Opaque identifier for a running simulation instance (IEEE HLA "federation").
/// Injected at startup via --federation-id; never discovered at runtime.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FederationId(pub String);

impl FederationId {
    /// Returns the federation ID as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(feature = "std")]
impl std::fmt::Display for FederationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Decoded IEEE 802.15.4 MAC Header (MHR) fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rf802154Mhr {
    /// Frame Control Field
    pub fcf: u16,
    /// Sequence Number
    pub seq_num: u8,
    /// Destination PAN ID
    pub dest_pan: u16,
    /// Destination Address (Short or Extended)
    pub dest_addr: u64,
    /// Source PAN ID
    pub src_pan: u16,
    /// Source Address (Short or Extended)
    pub src_addr: u64,
}

impl Rf802154Mhr {
    /// Parse a raw IEEE 802.15.4 frame into MHR fields.
    /// This is a best-effort parser for common frame types.
    pub fn parse(frame: &[u8]) -> Self {
        use byteorder::{ByteOrder, LittleEndian};

        const BROADCAST_PAN: u16 = 0xFFFF;
        const NO_ADDR: u64 = 0xFFFF_FFFF_FFFF_FFFF;
        const MIN_FRAME_LEN: usize = 3;
        const FCF_LEN: usize = 2;
        const ADDR_MODE_SHORT: u16 = 0x02;
        const ADDR_MODE_EXT: u16 = 0x03;
        const ADDR_MODE_MASK: u16 = 0x03;
        const DEST_ADDR_MODE_SHIFT: u32 = 10;
        const SRC_ADDR_MODE_SHIFT: u32 = 14;
        const PAN_ID_COMP_BIT: u16 = 1 << 6;
        const PAN_ID_LEN: usize = 2;
        const SHORT_ADDR_LEN: usize = 2;
        const EXT_ADDR_LEN: usize = 8;

        let mut mhr = Rf802154Mhr {
            fcf: 0,
            seq_num: 0,
            dest_pan: BROADCAST_PAN,
            dest_addr: NO_ADDR,
            src_pan: BROADCAST_PAN,
            src_addr: NO_ADDR,
        };

        if frame.len() < MIN_FRAME_LEN {
            return mhr;
        }

        mhr.fcf = LittleEndian::read_u16(&frame[0..FCF_LEN]);
        mhr.seq_num = frame[FCF_LEN];
        let dest_addr_mode = (mhr.fcf >> DEST_ADDR_MODE_SHIFT) & ADDR_MODE_MASK;
        let src_addr_mode = (mhr.fcf >> SRC_ADDR_MODE_SHIFT) & ADDR_MODE_MASK;
        let pan_id_comp = (mhr.fcf & PAN_ID_COMP_BIT) != 0;

        let mut offset = FCF_LEN + 1; // Skip FCF (2) and SeqNum (1)

        // Destination Addressing
        match dest_addr_mode {
            ADDR_MODE_SHORT if frame.len() >= offset + PAN_ID_LEN + SHORT_ADDR_LEN => {
                mhr.dest_pan = LittleEndian::read_u16(&frame[offset..offset + PAN_ID_LEN]);
                mhr.dest_addr = u64::from(LittleEndian::read_u16(
                    &frame[offset + PAN_ID_LEN..offset + PAN_ID_LEN + SHORT_ADDR_LEN],
                ));
                offset += PAN_ID_LEN + SHORT_ADDR_LEN;
            }
            ADDR_MODE_EXT if frame.len() >= offset + PAN_ID_LEN + EXT_ADDR_LEN => {
                mhr.dest_pan = LittleEndian::read_u16(&frame[offset..offset + PAN_ID_LEN]);
                mhr.dest_addr = LittleEndian::read_u64(
                    &frame[offset + PAN_ID_LEN..offset + PAN_ID_LEN + EXT_ADDR_LEN],
                );
                offset += PAN_ID_LEN + EXT_ADDR_LEN;
            }
            _ => {}
        }

        // Source Addressing
        match src_addr_mode {
            ADDR_MODE_SHORT => {
                if !pan_id_comp {
                    if frame.len() >= offset + PAN_ID_LEN {
                        mhr.src_pan = LittleEndian::read_u16(&frame[offset..offset + PAN_ID_LEN]);
                        offset += PAN_ID_LEN;
                    }
                } else {
                    mhr.src_pan = mhr.dest_pan;
                }
                if frame.len() >= offset + SHORT_ADDR_LEN {
                    mhr.src_addr =
                        u64::from(LittleEndian::read_u16(&frame[offset..offset + SHORT_ADDR_LEN]));
                }
            }
            ADDR_MODE_EXT => {
                if !pan_id_comp {
                    if frame.len() >= offset + PAN_ID_LEN {
                        mhr.src_pan = LittleEndian::read_u16(&frame[offset..offset + PAN_ID_LEN]);
                        offset += PAN_ID_LEN;
                    }
                } else {
                    mhr.src_pan = mhr.dest_pan;
                }
                if frame.len() >= offset + EXT_ADDR_LEN {
                    mhr.src_addr = LittleEndian::read_u64(&frame[offset..offset + EXT_ADDR_LEN]);
                }
            }
            _ => {}
        }

        mhr
    }
}

#[allow( // virtmcu-allow: allow reasoning="FlatBuffers-generated module"
    clippy::all, // virtmcu-allow: allow reasoning="FlatBuffers-generated module — machine-generated code, not hand-written"
    missing_docs,
    clippy::unwrap_used,
    clippy::missing_safety_doc,
    clippy::undocumented_unsafe_blocks,
    clippy::extra_unused_lifetimes
)]
pub mod telemetry_generated;
#[allow( // virtmcu-allow: allow reasoning="FlatBuffers-generated module"
    clippy::all, // virtmcu-allow: allow reasoning="FlatBuffers-generated module — machine-generated code, not hand-written"
    missing_docs,
    clippy::unwrap_used,
    clippy::missing_safety_doc,
    clippy::undocumented_unsafe_blocks,
    clippy::extra_unused_lifetimes
)]
pub mod wifi_generated;
pub use core_generated::virtmcu::core::*;

/// Errors that can occur during the virtmcu protocol handshake.
#[derive(Debug, thiserror::Error)]
pub enum HandshakeError {
    /// I/O error during handshake.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// The client sent an invalid magic value.
    #[error("bad magic: expected 0x{expected:08X}, got 0x{got:08X}")]
    BadMagic {
        /// The expected magic value.
        expected: u32,
        /// The magic value actually received.
        got: u32,
    },
    /// The client sent an unsupported protocol version.
    #[error("unsupported version: expected {expected}, got {got}")]
    BadVersion {
        /// The expected protocol version.
        expected: u32,
        /// The protocol version actually received.
        got: u32,
    },
}

/// Complete the **server** side of the virtmcu handshake on an async byte stream.
///
/// The client (QEMU mmio-socket-bridge) sends its handshake first; the server
/// reads it, validates magic and version, then replies with its own handshake.
///
/// # Errors
/// Returns [`HandshakeError`] if the I/O fails or the client sends an
/// unexpected magic value or protocol version.
#[cfg(feature = "tokio")]
pub async fn complete_server_handshake<S>(stream: &mut S) -> Result<(), HandshakeError>
where
    S: tokio::io::AsyncReadExt + tokio::io::AsyncWriteExt + Unpin,
{
    let mut buf = [0u8; VIRTMCU_HANDSHAKE_SIZE];
    stream.read_exact(&mut buf).await?;
    let client = VirtmcuHandshake::unpack_slice(&buf).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, "Failed to unpack handshake")
    })?;
    if client.magic() != VIRTMCU_PROTO_MAGIC {
        return Err(HandshakeError::BadMagic {
            expected: VIRTMCU_PROTO_MAGIC,
            got: client.magic(),
        });
    }
    if client.version() != VIRTMCU_PROTO_VERSION {
        return Err(HandshakeError::BadVersion {
            expected: VIRTMCU_PROTO_VERSION,
            got: client.version(),
        });
    }
    let server = VirtmcuHandshake::new(VIRTMCU_PROTO_MAGIC, VIRTMCU_PROTO_VERSION);
    stream.write_all(server.pack()).await?;
    Ok(())
}

/// Extension trait for FlatBuffer structs
pub trait FlatBufferStructExt: Sized {
    /// Unpack from a fixed-size byte array
    fn unpack(b: &[u8; 8]) -> Option<Self> {
        Self::unpack_slice(b)
    }
    /// Unpack slice
    fn unpack_slice(b: &[u8]) -> Option<Self>;
    /// Pack
    fn pack(&self) -> &[u8];
}
impl FlatBufferStructExt for VirtmcuHandshake {
    fn unpack(b: &[u8; 8]) -> Option<Self> {
        Some(Self(*b))
    }
    fn unpack_slice(b: &[u8]) -> Option<Self> {
        b.get(0..core::mem::size_of::<Self>())?.try_into().ok().map(Self)
    }
    fn pack(&self) -> &[u8] {
        &self.0
    }
}
impl FlatBufferStructExt for MmioReq {
    fn unpack_slice(b: &[u8]) -> Option<Self> {
        b.get(0..core::mem::size_of::<Self>())?.try_into().ok().map(Self)
    }
    fn pack(&self) -> &[u8] {
        &self.0
    }
}
impl FlatBufferStructExt for SyscMsg {
    fn unpack_slice(b: &[u8]) -> Option<Self> {
        b.get(0..core::mem::size_of::<Self>())?.try_into().ok().map(Self)
    }
    fn pack(&self) -> &[u8] {
        &self.0
    }
}
impl FlatBufferStructExt for ClockAdvanceReq {
    fn unpack_slice(b: &[u8]) -> Option<Self> {
        b.get(0..core::mem::size_of::<Self>())?.try_into().ok().map(Self)
    }
    fn pack(&self) -> &[u8] {
        &self.0
    }
}
impl FlatBufferStructExt for ClockReadyResp {
    fn unpack_slice(b: &[u8]) -> Option<Self> {
        b.get(0..core::mem::size_of::<Self>())?.try_into().ok().map(Self)
    }
    fn pack(&self) -> &[u8] {
        &self.0
    }
}
impl FlatBufferStructExt for ZenohFrameHeader {
    fn unpack_slice(b: &[u8]) -> Option<Self> {
        b.get(0..core::mem::size_of::<Self>())?.try_into().ok().map(Self)
    }
    fn pack(&self) -> &[u8] {
        &self.0
    }
}
impl FlatBufferStructExt for ZenohSPIHeader {
    fn unpack_slice(b: &[u8]) -> Option<Self> {
        b.get(0..core::mem::size_of::<Self>())?.try_into().ok().map(Self)
    }
    fn pack(&self) -> &[u8] {
        &self.0
    }
}

/// A constant
pub const VIRTMCU_PROTO_MAGIC: u32 = 0x564D4355;
/// A constant
pub const VIRTMCU_PROTO_VERSION: u32 = 1;

/// Maximum wait for the first RTI quantum advance.
/// Covers firmware compilation (arm-none-eabi-gcc, 30–90 s) and QEMU JIT warm-up.
/// Matches the boot timeout in the QEMU zenoh-clock plugin.
pub const BOOT_QUANTUM_TIMEOUT: core::time::Duration = core::time::Duration::from_secs(300);

/// Maximum wait for a steady-state RTI quantum advance.
/// If no advance arrives within this window the simulation is considered hung.
pub const NORMAL_QUANTUM_TIMEOUT: core::time::Duration = core::time::Duration::from_secs(10);

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

/// Abstract transport for clock synchronization between Physical Node and node.
pub trait ClockSyncTransport: Send + Sync {
    /// Blocks until a clock advancement request is received, a timeout occurs, or transport is closed.
    /// Returns the request and a responder trait object.
    fn recv_advance(
        &self,
        timeout: core::time::Duration,
    ) -> Option<(ClockAdvanceReq, Box<dyn ClockSyncResponder>)>;

    /// Sends a virtual time heartbeat signal to external listeners.
    fn send_vtime_heartbeat(&self, _vtime_ns: u64) {}
}

/// Abstract responder for a specific clock advancement request.
pub trait ClockSyncResponder: Send + Sync {
    /// Sends a clock ready response back to the Physical Node.
    fn send_ready(&self, resp: ClockReadyResp) -> Result<(), alloc::string::String>;
}

/// Abstract transport for clock synchronization from the Physical Node perspective.
pub trait PhysicalNodeTransport: Send + Sync {
    /// Send a clock-advance request and block until the node replies.
    /// Returns the reply, or None on timeout/transport error.
    fn advance(
        &self,
        req: ClockAdvanceReq,
        timeout: core::time::Duration,
    ) -> Option<ClockReadyResp>;
}

/// Abstract transport for the Physical Node ↔ Physics Gateway handshake.
///
/// The Physical Node sends a trigger containing all actuator data for the
/// completed quantum and blocks until the gateway responds with a done signal.
/// Implementations: ZenohPhysicsTransport, UnixSocketPhysicsTransport.
pub trait PhysicsGatewayTransport: Send + Sync {
    /// Send the complete actuator bundle for one quantum to the gateway and
    /// block until the gateway signals that the physics step is complete.
    ///
    /// Returns `Err` on transport failure, timeout, or a non-OK status in the
    /// `PhysicsDone` response.  Callers must treat any `Err` as fatal.
    fn trigger_and_wait(
        &self,
        trigger_bytes: &[u8],
        timeout: core::time::Duration,
    ) -> Result<(), alloc::string::String>;
}

/// Server-side counterpart: implemented by the Physics Gateway to receive
/// triggers and send done signals.
pub trait PhysicsGatewayServer: Send + Sync {
    /// Block until a trigger arrives.  Returns the raw FlatBuffers bytes of
    /// the `PhysicsTrigger` table, or `None` on shutdown/transport close.
    fn recv_trigger(&self, timeout: core::time::Duration) -> Option<alloc::vec::Vec<u8>>;

    /// Send the done signal back to the Physical Node.
    fn send_done(&self, done: physics_proto::PhysicsDone) -> Result<(), alloc::string::String>;
}

/// Causal actuator commands for one quantum.
///
/// Outer key: delivery virtual time (ns) at which firmware issued the command.
/// Inner key: actuator index as declared in the board topology YAML.
/// Inner value: actuator data words (length = `data_size` from topology).
///
/// Use `BTreeMap` (not `HashMap`) throughout — this type must be `no_std`-compatible.
pub type ActuatorMap =
    alloc::collections::BTreeMap<u64, alloc::collections::BTreeMap<u32, alloc::vec::Vec<f64>>>;

/// Sensor readings produced by one quantum step of the physical plant.
///
/// Key: sensor index as declared in the board topology YAML.
/// Value: sensor data words (length = `data_size` from topology).
///
/// Empty for `RemotePlant` — the Physics Gateway publishes sensors directly.
pub type SensorMap = alloc::collections::BTreeMap<u32, alloc::vec::Vec<f64>>;

/// State produced by one quantum step of the physical plant.
pub struct PlantState {
    /// Virtual time (ns) at the END of the completed quantum.
    pub vtime_ns: u64,
    /// Sensor readings to publish on `sim/sensor/{node}/sensordata_{i}`.
    ///
    /// Empty when the plant delegates to an external Physics Gateway process,
    /// which publishes sensors directly. Non-empty for in-process (`EmbeddedPlant`).
    pub sensors: SensorMap,
}

/// The physical world in a Cyber-Physical System simulation.
///
/// Owns virtual time progression and plant dynamics for exactly one simulation node.
/// The binary's main loop calls `step()` once per quantum, after:
/// 1. Issuing `ClockAdvanceReq` and receiving `ClockReadyResp` from the QEMU cyber node.
/// 2. Draining the `ZenohActuatorSink` to obtain the complete actuator bundle.
///
/// Implementations: `TickOnlyPlant`, `EmbeddedPlant`, `RemotePlant`.
pub trait PhysicalNode: Send + Sync {
    /// Advance the plant by one quantum.
    ///
    /// `quantum_ns` is the size of the completed quantum in nanoseconds.
    /// `actuators` contains all firmware commands delivered during this quantum,
    /// ordered by `(delivery_vtime_ns, actuator_id)`.
    ///
    /// Returns the updated `PlantState` or a fatal error string. Callers treat
    /// any `Err` as a simulation abort — do not retry.
    fn step(
        &mut self,
        quantum_ns: u64,
        actuators: &ActuatorMap,
    ) -> Result<PlantState, alloc::string::String>;
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
}

/// Unix socket-based Physical Node transport.
#[cfg(feature = "std")]
pub struct UnixSocketPhysicalNodeTransport {
    listener: std::os::unix::net::UnixListener,
    stream: std::sync::Mutex<Option<std::os::unix::net::UnixStream>>,
}

#[cfg(feature = "std")]
impl UnixSocketPhysicalNodeTransport {
    /// Creates a new `UnixSocketPhysicalNodeTransport` that listens on the given path.
    ///
    /// If the socket file already exists, it will be removed.
    pub fn new<P: AsRef<std::path::Path>>(path: P) -> std::io::Result<Self> {
        let path = path.as_ref();
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        let listener = std::os::unix::net::UnixListener::bind(path)?;
        Ok(Self { listener, stream: std::sync::Mutex::new(None) })
    }

    fn ensure_stream(&self) -> Option<std::os::unix::net::UnixStream> {
        let mut guard = self.stream.lock().ok()?;
        if let Some(stream) = &*guard {
            return stream.try_clone().ok();
        }

        // Accept a new connection (blocks until QEMU connects)
        let (stream, _) = self.listener.accept().ok()?;
        *guard = Some(stream.try_clone().ok()?);
        Some(stream)
    }
}

#[cfg(feature = "std")]
impl PhysicalNodeTransport for UnixSocketPhysicalNodeTransport {
    fn advance(
        &self,
        req: ClockAdvanceReq,
        timeout: core::time::Duration,
    ) -> Option<ClockReadyResp> {
        use std::io::{Read, Write};

        let mut stream = self.ensure_stream()?;
        let _ = stream.set_write_timeout(Some(timeout));
        let _ = stream.set_read_timeout(Some(timeout));

        let bytes = req.pack();
        if stream.write_all(bytes).is_err() {
            // Broken pipe? Try re-accepting
            let mut guard = self.stream.lock().ok()?;
            *guard = None;
            drop(guard);
            stream = self.ensure_stream()?;
            let _ = stream.set_write_timeout(Some(timeout));
            let _ = stream.set_read_timeout(Some(timeout));
            if stream.write_all(bytes).is_err() {
                return None;
            }
        }

        let mut buf = [0u8; 24];
        if stream.read_exact(&mut buf).is_err() {
            // If read fails, the connection might be dead. Clear it.
            if let Ok(mut guard) = self.stream.lock() {
                *guard = None;
            }
            return None;
        }

        ClockReadyResp::unpack_slice(&buf)
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
    /// A struct field
    pub power_uw: u32,
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

/// Encode a `ZenohFrameHeader` + payload into a byte vector (little-endian).
/// Encode a `ZenohFrameHeader` + payload into a byte vector (little-endian).
pub fn encode_frame(delivery_vtime_ns: u64, sequence_number: u64, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(ZENOH_FRAME_HEADER_SIZE + payload.len());
    let header = ZenohFrameHeader::new(delivery_vtime_ns, sequence_number, payload.len() as u32);
    out.extend_from_slice(&header.0);
    out.extend_from_slice(payload);
    out
}

/// Encode an `Rf802154Frame` (including payload) into a byte vector (little-endian, size-prefixed).
pub fn encode_rf802154_frame(
    delivery_vtime_ns: u64,
    sequence_number: u64,
    payload: &[u8],
    rssi: i8,
    lqi: u8,
    mhr: Rf802154Mhr,
) -> Vec<u8> {
    const INITIAL_CAPACITY: usize = 128;
    let mut builder = flatbuffers::FlatBufferBuilder::with_capacity(INITIAL_CAPACITY);
    let data = builder.create_vector(payload);

    let args = rf802154::Rf802154FrameArgs {
        delivery_vtime_ns,
        sequence_number,
        rssi,
        lqi,
        fcf: mhr.fcf,
        mhr_seq_num: mhr.seq_num,
        dest_pan: mhr.dest_pan,
        dest_addr: mhr.dest_addr,
        src_pan: mhr.src_pan,
        src_addr: mhr.src_addr,
        data: Some(data),
    };

    let frame = rf802154::Rf802154Frame::create(&mut builder, &args);
    builder.finish_size_prefixed(frame, None);
    builder.finished_data().to_vec()
}

/// Decode a `ZenohFrameHeader` from the first 20 bytes of `data`.
///
/// Returns `None` if `data` is shorter than `ZENOH_FRAME_HEADER_SIZE`.
/// Decode a `ZenohFrameHeader` from the first 24 bytes of `data`.
pub fn decode_frame(data: &[u8]) -> Option<(ZenohFrameHeader, &[u8])> {
    let header_bytes = data.get(..ZENOH_FRAME_HEADER_SIZE)?;
    let remaining = data.get(ZENOH_FRAME_HEADER_SIZE..)?;
    let mut buf = [0u8; 24];
    buf.copy_from_slice(header_bytes);
    let header = ZenohFrameHeader(buf);
    Some((header, remaining))
}

/// Callback type for data transport subscriptions.
pub type DataCallback = Box<dyn Fn(&str, &[u8]) + Send + Sync>;

/// Abstract transport for emulated data plane (packets, signals).
///
/// This trait abstracts the underlying communication mechanism (e.g., Zenoh, Unix Sockets)
/// used for peripheral data traffic.
/// Represents an active liveliness declaration on the network.
pub trait LivelinessToken: Send + Sync {
    /// Drops or explicitly undeclares the liveliness token.
    fn drop_token(&mut self) {}
}

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

    /// Declares liveliness on the network for a specific topic.
    fn declare_liveliness(&self, _topic: &str) -> Option<alloc::boxed::Box<dyn LivelinessToken>> {
        None
    }

    /// Closes the transport.
    fn close(&self) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic, clippy::magic_numbers)] // virtmcu-allow: allow reasoning="Legacy test module exceptions"
mod tests {
    use super::*;

    // ── Delivery queue ordering (mirrors chardev/netdev OrderedPacket)
    // chardev uses BinaryHeap<OrderedPacket> as a min-heap by vtime + sequence.
    // The Ord impl inverts comparison so the heap pops the lowest vtime first.
    // These tests validate the invariant without needing QEMU FFI.

    use alloc::collections::BinaryHeap;
    use core::cmp::Ordering as CmpOrd;

    const TEST_VTIME_1000: u64 = 1_000;
    const TEST_VTIME_500: u64 = 500;
    const TEST_SEQ_2: u64 = 2;
    const TEST_SEQ_42: u64 = 42;
    const TEST_VAL_123: u32 = 123;
    const TEST_VTIME_LONG: u64 = 12345678;
    const TEST_VTIME_10M: u64 = 10_000_000;
    const TEST_QUANTUM_99: u64 = 99;
    const TEST_N_FRAMES_50: u32 = 50;
    const TEST_ADDR_1000: u64 = 0x1000_0000;
    const TEST_DATA_DEADBEEF: u64 = 0xDEAD_BEEF;
    const TEST_VAL_U16_1234: u16 = 0x1234;
    const TEST_VAL_U32_5678: u32 = 0x56789ABC;
    const TEST_IRQ_7: u32 = 7;
    const TEST_PATTERN_U64: u64 = 0x0102030405060708;
    const TEST_PATTERN_U64_ALT: u64 = 0x0A0B0C0D0E0F1011;
    const TEST_EXPECTED_LEN_2: usize = 2;
    const TEST_EXPECTED_LEN_5: usize = 5;
    const TEST_FRAME_SIZE: usize = 24;
    const TEST_LE_PATTERN: [u8; 8] = [0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01];
    const TEST_PATTERN_U64_MMIO_1: u64 = 0x99AABBCCDDEEFF00;
    const TEST_PATTERN_U64_MMIO_2: u64 = 0x1020304050607080;
    const TEST_LE_PATTERN_ALT: [u8; 8] = [0x11, 0x10, 0x0F, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A];
    const NODE_ID_3: &str = "3";
    #[rustfmt::skip]
    const EXPECTED_MMIO_BYTES: [u8; 32] = [0x01, 0x04, 0x34, 0x12, 0xBC, 0x9A, 0x78, 0x56, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11, 0x00, 0xFF, 0xEE, 0xDD, 0xCC, 0xBB, 0xAA, 0x99, 0x80, 0x70, 0x60, 0x50, 0x40, 0x30, 0x20, 0x10];

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
        heap.push(TestPacket { vtime: TEST_VTIME_1000, sequence: TEST_SEQ_2 });
        heap.push(TestPacket { vtime: TEST_VTIME_1000, sequence: 0 });
        heap.push(TestPacket { vtime: TEST_VTIME_1000, sequence: 1 });
        assert_eq!(heap.pop().ok_or("Empty")?.sequence, 0);
        assert_eq!(heap.pop().ok_or("Empty")?.sequence, 1);
        assert_eq!(heap.pop().ok_or("Empty")?.sequence, TEST_SEQ_2);
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
        heap.push(TestPacket { vtime: TEST_VTIME_500, sequence: 0 });
        heap.push(TestPacket { vtime: TEST_VTIME_500, sequence: 1 });
        assert_eq!(heap.len(), TEST_EXPECTED_LEN_2);
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
        let b = TestPacket { vtime: TEST_SEQ_2, sequence: 0 };
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
        assert_eq!(
            format!("sim/clock/advance/{}", NODE_ID_3),
            format!("sim/clock/advance/{}", NODE_ID_3)
        );
    }

    #[test]
    fn test_multi_node_chardev_isolation() {
        let base = "sim/chardev";
        let rx0 = format!("{base}/0/rx");
        let rx1 = format!("{base}/1/rx");
        assert_ne!(rx0, rx1, "node 0 and node 1 must use different topics");
    }

    // ── Struct size assertions ────────────────────────────────────────────────

    // Removed hardcoded FFI size tests as they are dynamic via flatc

    // ── Wire format: ZenohFrameHeader ────────────────────────────────────────

    #[test]
    fn test_encode_decode_round_trip() -> Result<(), String> {
        let payload = b"hello";
        let frame = encode_frame(TEST_VTIME_LONG, TEST_SEQ_42, payload);
        assert_eq!(frame.len(), TEST_FRAME_SIZE + TEST_EXPECTED_LEN_5);

        let (hdr, rest) = decode_frame(&frame).ok_or("Decode failed")?;
        assert_eq!({ hdr.delivery_vtime_ns() }, TEST_VTIME_LONG);
        assert_eq!({ hdr.sequence_number() }, TEST_SEQ_42);
        assert_eq!({ hdr.size() }, TEST_EXPECTED_LEN_5 as u32);
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

    const SLICE_0_8: core::ops::Range<usize> = 0..8;
    const SLICE_8_16: core::ops::Range<usize> = 8..16;
    const SLICE_16_20: core::ops::Range<usize> = 16..20;
    const SLICE_16_24: core::ops::Range<usize> = 16..24;

    #[test]
    fn test_little_endian_vtime() {
        // 0x0102030405060708 in LE = bytes [08, 07, 06, 05, 04, 03, 02, 01]
        let vtime: u64 = TEST_PATTERN_U64;
        let frame = encode_frame(vtime, 0, b"");
        assert_eq!(&frame[SLICE_0_8], &TEST_LE_PATTERN);
    }

    #[test]
    fn test_little_endian_sequence() {
        let seq: u64 = TEST_PATTERN_U64;
        let frame = encode_frame(0, seq, b"");
        assert_eq!(&frame[SLICE_8_16], &TEST_LE_PATTERN);
    }

    #[test]
    fn test_little_endian_size() {
        // size = 0x00000005 in LE = bytes [05, 00, 00, 00]
        let frame = encode_frame(0, 0, b"hello");
        const LE_SIZE_5: [u8; 4] = [0x05, 0x00, 0x00, 0x00];
        assert_eq!(&frame[SLICE_16_20], &LE_SIZE_5);
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
        const DIVISOR: u64 = 1_250_000;
        const BAUD_10MBPS_NS: u64 = 1_000_000_000 / DIVISOR;
        const TEST_VTIME_800: u64 = 800;
        assert_eq!(BAUD_10MBPS_NS, TEST_VTIME_800);
    }

    #[test]
    fn test_encode_decode_sequence_monotonic() -> Result<(), String> {
        const TEST_VTIME_800: u64 = 800;
        const N: u64 = 1_000;
        const START: u64 = 10_000_000;
        for i in 0..N {
            let vtime = START + i * TEST_VTIME_800;
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
        let req = ClockAdvanceReq::new(TEST_VTIME_10M, TEST_SEQ_42, TEST_VAL_123 as u64);
        let bytes = req.pack();
        let req2 = ClockAdvanceReq::unpack_slice(bytes).expect("API conversion failed");
        assert_eq!({ req.delta_ns() }, { req2.delta_ns() });
        assert_eq!({ req.absolute_vtime_ns() }, { req2.absolute_vtime_ns() });
        assert_eq!({ req.quantum_number() }, { req2.quantum_number() });
    }

    #[test]
    fn test_clock_advance_req_le_encoding() {
        let req = ClockAdvanceReq::new(TEST_PATTERN_U64, 0, TEST_PATTERN_U64_ALT);
        let bytes = req.pack();
        assert_eq!(&bytes[SLICE_0_8], &TEST_LE_PATTERN);
        assert_eq!(&bytes[SLICE_16_24], &TEST_LE_PATTERN_ALT);
    }

    #[test]
    fn test_clock_advance_req_zero() {
        let req = ClockAdvanceReq::new(0, 0, 0);
        let bytes = req.pack();
        assert_eq!(bytes, [0u8; TEST_FRAME_SIZE]);
    }

    // ── Wire format: ClockReadyResp ───────────────────────────────────────────

    #[test]
    fn test_clock_ready_resp_ok() {
        let resp =
            ClockReadyResp::new(TEST_VTIME_10M, TEST_N_FRAMES_50, CLOCK_ERROR_OK, TEST_QUANTUM_99);
        let bytes = resp.pack();
        let resp2 = ClockReadyResp::unpack_slice(bytes).expect("API conversion failed");
        assert_eq!({ resp2.current_vtime_ns() }, TEST_VTIME_10M);
        assert_eq!({ resp2.n_frames() }, TEST_N_FRAMES_50);
        assert_eq!({ resp2.error_code() }, CLOCK_ERROR_OK);
        assert_eq!({ resp2.quantum_number() }, TEST_QUANTUM_99);
    }

    #[test]
    fn test_clock_ready_resp_stall() {
        let resp = ClockReadyResp::new(0, 0, CLOCK_ERROR_STALL, 0);
        let bytes = resp.pack();
        let resp2 = ClockReadyResp::unpack_slice(bytes).expect("API conversion failed");
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
        const TEST_SIZE_4: u8 = 4;
        const TEST_PATTERN_U64_MMIO_3: u64 = 0x1122334455667788;
        let req = MmioReq::new(
            1,
            TEST_SIZE_4,
            TEST_VAL_U16_1234,
            TEST_VAL_U32_5678,
            TEST_PATTERN_U64_MMIO_3,
            TEST_PATTERN_U64_MMIO_1,
            TEST_PATTERN_U64_MMIO_2,
        );
        let bytes = req.pack();

        assert_eq!(
            bytes, EXPECTED_MMIO_BYTES,
            "Rust pack() output must exactly match Python struct.pack('<BBHIQQQ')"
        );
    }

    #[test]
    fn test_mmio_req_round_trip() {
        const TEST_SIZE_4: u8 = 4;
        const TEST_VTIME_999: u64 = 999_999;
        let req = MmioReq::new(
            MMIO_REQ_WRITE,
            TEST_SIZE_4,
            0,
            0,
            TEST_VTIME_999,
            TEST_ADDR_1000,
            TEST_DATA_DEADBEEF,
        );
        let bytes = req.pack();
        let req2 = MmioReq::unpack_slice(bytes).expect("API conversion failed");
        assert_eq!({ req2.type_() }, MMIO_REQ_WRITE);
        assert_eq!({ req2.size() }, TEST_SIZE_4);
        assert_eq!({ req2.vtime_ns() }, TEST_VTIME_999);
        assert_eq!({ req2.addr() }, TEST_ADDR_1000);
        assert_eq!({ req2.data() }, TEST_DATA_DEADBEEF);
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
        let msg = SyscMsg::new(SYSC_MSG_IRQ_SET, TEST_IRQ_7, 1);
        let bytes = msg.pack();
        let msg2 = SyscMsg::unpack_slice(bytes).expect("API conversion failed");
        assert_eq!({ msg2.type_() }, SYSC_MSG_IRQ_SET);
        assert_eq!({ msg2.irq_num() }, TEST_IRQ_7);
        assert_eq!({ msg2.data() }, 1u64);
    }

    // ── Proto magic / version ─────────────────────────────────────────────────

    #[test]
    fn test_proto_magic_value() {
        // VIRTMCU_PROTO_MAGIC = 0x564D4355
        assert_eq!(VIRTMCU_PROTO_MAGIC, 0x564D_4355);
        // In little-endian bytes on wire: [0x55, 0x43, 0x4D, 0x56] = "UCMV"
        let bytes = VIRTMCU_PROTO_MAGIC.to_le_bytes();
        const EXPECTED_BYTES: [u8; 4] = [0x55, 0x43, 0x4D, 0x56];
        assert_eq!(bytes, EXPECTED_BYTES);
    }

    #[test]
    fn test_proto_version_is_one() {
        assert_eq!(VIRTMCU_PROTO_VERSION, 1);
    }

    #[test]
    fn test_handshake_round_trip() {
        let hs = VirtmcuHandshake::new(VIRTMCU_PROTO_MAGIC, VIRTMCU_PROTO_VERSION);
        let bytes = hs.pack();
        let hs2 = VirtmcuHandshake::unpack_slice(bytes).expect("API conversion failed");
        assert_eq!({ hs2.magic() }, VIRTMCU_PROTO_MAGIC);
        assert_eq!({ hs2.version() }, VIRTMCU_PROTO_VERSION);
    }

    #[test]
    fn test_header_roundtrip() {
        const TEST_VTIME: u64 = 12345;
        const TEST_SEQ: u64 = 7;
        const TEST_SIZE: u64 = 100;
        let h = ZenohFrameHeader::new(TEST_VTIME, TEST_SEQ, TEST_SIZE.try_into().unwrap());
        let bytes = h.pack();
        let h2 = ZenohFrameHeader::unpack_slice(bytes).expect("API conversion failed");
        assert_eq!({ h.delivery_vtime_ns() }, { h2.delivery_vtime_ns() });
        assert_eq!({ h.sequence_number() }, { h2.sequence_number() });
        assert_eq!({ h.size() }, { h2.size() });
    }

    #[test]
    fn test_zenoh_frame_header_size() {
        const HDR_TOTAL_SIZE: usize = 24;
        assert_eq!(core::mem::size_of::<ZenohFrameHeader>(), HDR_TOTAL_SIZE);
        assert_eq!(ZENOH_FRAME_HEADER_SIZE, HDR_TOTAL_SIZE);
    }

    #[test]
    fn test_header_le_bytes() {
        let h = ZenohFrameHeader::new(1, 0, 0);
        let bytes = h.pack();
        assert_eq!(bytes[0], 1);
        assert_eq!(bytes[1], 0);
    }

    #[test]
    fn test_header_seq_ordering() {
        const TEST_VTIME: u64 = 100;
        const SEQ0: u64 = 0;
        const SEQ1: u64 = 1;
        const SEQ2: u64 = 2;
        const SEQ3: u64 = 3;
        const SEQ4: u64 = 4;
        let mut frames = alloc::vec![
            ZenohFrameHeader::new(TEST_VTIME, SEQ4, 0),
            ZenohFrameHeader::new(TEST_VTIME, SEQ2, 0),
            ZenohFrameHeader::new(TEST_VTIME, SEQ0, 0),
            ZenohFrameHeader::new(TEST_VTIME, SEQ3, 0),
            ZenohFrameHeader::new(TEST_VTIME, SEQ1, 0),
        ];
        frames.sort_by_key(super::core_generated::virtmcu::core::ZenohFrameHeader::sequence_number);
        let seqs: alloc::vec::Vec<u64> = frames
            .iter()
            .map(super::core_generated::virtmcu::core::ZenohFrameHeader::sequence_number)
            .collect();
        const EXPECTED_SEQS: [u64; 5] = [SEQ0, SEQ1, SEQ2, SEQ3, SEQ4];
        assert_eq!(seqs, EXPECTED_SEQS);
    }

    #[test]
    fn test_virtmcu_handshake_unpack_error() {
        const INVALID_DATA: &[u8] = b"1234";
        assert!(VirtmcuHandshake::unpack_slice(INVALID_DATA).is_none());
    }

    #[test]
    fn test_mmio_req_unpack_error() {
        const INVALID_DATA: &[u8] = b"1234567890";
        assert!(MmioReq::unpack_slice(INVALID_DATA).is_none());
    }

    #[test]
    fn test_sysc_msg_unpack_error() {
        assert!(SyscMsg::unpack_slice(b"short").is_none());
    }

    #[test]
    fn test_clock_advance_req_unpack_error() {
        assert!(ClockAdvanceReq::unpack_slice(b"toolittle").is_none());
    }

    #[test]
    fn test_clock_ready_resp_unpack_error() {
        assert!(ClockReadyResp::unpack_slice(b"wrongsize").is_none());
    }

    #[test]
    fn test_zenoh_frame_header_unpack_error() {
        assert!(ZenohFrameHeader::unpack_slice(b"short").is_none());
    }

    #[test]
    fn test_zenoh_spi_header_unpack_error() {
        assert!(ZenohSPIHeader::unpack_slice(b"short").is_none());
    }

    #[cfg(all(test, feature = "tokio"))]
    #[allow(clippy::magic_numbers)] // virtmcu-allow: allow reasoning="Tests require specific magic numbers"
    mod handshake_server_tests {
        use super::*;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        #[tokio::test]
        async fn test_valid_handshake_accepted() {
            const BUFFER_SIZE: usize = 1024;
            let (mut client, mut server) = tokio::io::duplex(BUFFER_SIZE);

            let hs_client = VirtmcuHandshake::new(VIRTMCU_PROTO_MAGIC, VIRTMCU_PROTO_VERSION);
            client.write_all(hs_client.pack()).await.expect("API conversion failed");

            complete_server_handshake(&mut server).await.expect("API conversion failed");

            let mut hs_server_bytes = [0u8; VIRTMCU_HANDSHAKE_SIZE];
            client.read_exact(&mut hs_server_bytes).await.expect("API conversion failed");
            let hs_server =
                VirtmcuHandshake::unpack_slice(&hs_server_bytes).expect("API conversion failed");
            assert_eq!(hs_server.magic(), VIRTMCU_PROTO_MAGIC);
            assert_eq!(hs_server.version(), VIRTMCU_PROTO_VERSION);
        }

        #[tokio::test]
        async fn test_bad_magic_rejected() {
            const BUFFER_SIZE: usize = 1024;
            let (mut client, mut server) = tokio::io::duplex(BUFFER_SIZE);

            const BAD_MAGIC: u32 = 0xDEADBEEF;
            let hs_client = VirtmcuHandshake::new(BAD_MAGIC, VIRTMCU_PROTO_VERSION);
            client.write_all(hs_client.pack()).await.expect("API conversion failed");

            let res = complete_server_handshake(&mut server).await;
            match res {
                Err(HandshakeError::BadMagic { expected, got }) => {
                    assert_eq!(expected, VIRTMCU_PROTO_MAGIC);
                    assert_eq!(got, BAD_MAGIC);
                }
                _ => panic!("Expected BadMagic error, got {:?}", res),
            }
        }

        #[tokio::test]
        async fn test_bad_version_rejected() {
            const BUFFER_SIZE: usize = 1024;
            let (mut client, mut server) = tokio::io::duplex(BUFFER_SIZE);

            const BAD_VERSION: u32 = 99;
            let hs_client = VirtmcuHandshake::new(VIRTMCU_PROTO_MAGIC, BAD_VERSION);
            client.write_all(hs_client.pack()).await.expect("API conversion failed");

            let res = complete_server_handshake(&mut server).await;
            match res {
                Err(HandshakeError::BadVersion { expected, got }) => {
                    assert_eq!(expected, VIRTMCU_PROTO_VERSION);
                    assert_eq!(got, BAD_VERSION);
                }
                _ => panic!("Expected BadVersion error, got {:?}", res),
            }
        }
    }
}

#[cfg(test)]
mod timeout_tests {
    use super::*;
    use core::time::Duration;

    #[test]
    fn test_boot_timeout_is_sane() {
        const MIN_BOOT_TIMEOUT_SECS: u64 = 60;
        assert!(BOOT_QUANTUM_TIMEOUT >= Duration::from_secs(MIN_BOOT_TIMEOUT_SECS));
    }

    #[test]
    fn test_normal_timeout_is_sane() {
        const MIN_NORMAL_TIMEOUT_SECS: u64 = 5;
        assert!(NORMAL_QUANTUM_TIMEOUT >= Duration::from_secs(MIN_NORMAL_TIMEOUT_SECS));
    }

    #[test]
    fn test_boot_exceeds_normal() {
        assert!(BOOT_QUANTUM_TIMEOUT > NORMAL_QUANTUM_TIMEOUT);
    }
}
