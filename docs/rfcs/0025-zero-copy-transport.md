# RFC-0025: Zero-Copy Deterministic Transport API

## Status
Accepted

## Context & Problem Statement
VirtMCU currently relies on `DataTransport::publish(&self, topic, payload: &[u8])`. While this interface is simple, it forces a memory allocation and a memory copy for every single packet transmitted. When running high-density simulations (e.g., 50 nodes communicating over virtual CAN bus at 1Mbit/s), the overhead of allocating `Vec<u8>` for every frame becomes a significant bottleneck.

Furthermore, we need to support two distinct transport layers transparently:
1. **Zenoh:** A distributed Pub/Sub network for scaling simulations across multiple physical host machines (high latency, network-backed).
2. **Unix/SHM (Shared Memory):** A hyper-fast local transport for running the entire cluster on a single massive multicore server (sub-microsecond latency, zero-copy).

To achieve "State of the Art" (SOTA) simulation speed while maintaining strict PDES determinism, we need an abstraction that allows peripheral models to write data *directly* into the transport's memory buffer (Zero-Copy) without knowing whether that buffer is backed by a Zenoh TCP socket or a POSIX shared memory ring buffer.

## Proposed "Reservation" API
We propose deprecating `publish(payload)` in favor of a **Transport-Agnostic Reservation API**. 

Instead of passing a buffer to the transport, the peripheral *requests* a buffer from the transport, writes to it, and commits it.

```rust
pub trait DataTransport: Send + Sync {
    // ...

    /// Requests a zero-copy buffer from the transport layer.
    /// In SHM, this returns a mutable slice pointing directly into the ring buffer.
    /// In Zenoh, this allocates a buffer that will later be handed to the socket.
    fn reserve(&self, topic: &str, size: usize) -> Result<TransportReservation, TransportError>;
}

/// A zero-copy buffer reserved in the transport layer.
pub struct TransportReservation<'a> {
    buffer: &'a mut [u8],
    topic: &'a str,
    // ... internal transport tracking ...
}

impl<'a> TransportReservation<'a> {
    /// Allows the peripheral to write directly into the transport buffer.
    pub fn buffer_mut(&mut self) -> &mut [u8] {
        self.buffer
    }

    /// Commits the buffer, making it visible to subscribers.
    /// For SHM, this is a lock-free atomic pointer swap.
    /// For Zenoh, this dispatches the buffer to the background network thread.
    pub fn commit(self, delivery_vtime_ns: u64, sequence_number: u64) -> Result<(), TransportError>;
}
```

### Flow Example in a Peripheral (e.g., `reference-peripheral`)

```rust
// Old Pattern (1 memory allocation, 1 memory copy):
let payload = val.to_le_bytes();
let frame = virtmcu_wire::encode_frame(0, 0, &payload); 
transport.publish(&topic, &frame);

// New SOTA Pattern (Zero allocations, zero copies on the hot path):
let mut reservation = transport.reserve(&topic, 4)?;
reservation.buffer_mut().copy_from_slice(&val.to_le_bytes());
reservation.commit(0, 0)?; // The frame header is written implicitly by the transport!
```

## Amendment: UDS + Thread-Local Arenas

*Adopted after initial SHM prototype. Original proposal above is unchanged.*

While this RFC initially proposed backing the Reservation API with Shared Memory (SHM) ring buffers for single-host simulation, analysis revealed critical robustness issues with SHM for event transport (e.g., "zombie node" deadlocks and manual routing complexity, see RFC-0019).

Instead, VirtMCU adopted the **API design** of this RFC, but backed it with the **Unix Domain Sockets (UDS)** implementation mandated by RFC-0019.

**How it works:**
1. **Thread-Local Arenas:** `reserve()` provides a mutable reference (`&mut [u8]`) to a thread-local, pre-allocated arena. Because each vCPU thread owns its arena, reservations are entirely lock-free (no Mutexes, no atomic CAS).
2. **Compile-Time Safety:** The Rust borrow checker enforces the `TransportReservation<'a>` lifetime, statically guaranteeing that the peripheral cannot hold a reference to the buffer past `commit()`.
3. **Kernel Serialization:** `commit()` performs a single `write()` system call to push the arena buffer down the UDS file descriptor. The Linux kernel natively handles queueing, ordering, and serialization.

This hybrid approach eliminates memory allocations in the peripheral while leveraging the OS kernel for robust IPC lifecycle management (Fail Loudly on socket closure).



## Consequences
* **Positive:** Complete elimination of `encode_frame` boilerplate inside peripheral business logic.
* **Positive:** Reduced garbage collector pressure / heap fragmentation via zero-allocation thread-local arenas.
* **Positive:** Compile-time memory safety enforced by the Rust borrow checker.
* **Positive:** Future-proofs peripheral models. If SHM is ever required, only the transport backend changes.
* **Negative:** The `reserve()` call can fail (e.g., if the UDS socket is closed), forcing peripheral developers to handle `TransportError` (enforcing Fail Loudly).

## Related
* RFC-0023: Safe QOM Macros
* RFC-0024: Assertion-Based Deterministic Routing
