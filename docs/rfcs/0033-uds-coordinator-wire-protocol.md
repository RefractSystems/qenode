# RFC-0033: UDS Coordinator Wire Protocol

## Status
Accepted (updated by RFC-0042 — see deprecated topics below)

## Context

RFC-0019 established Unix Domain Sockets (UDS) as the default single-host transport
for VirtMCU simulations, but left the on-the-wire framing, registration handshake,
and quantum-start signalling underspecified. The original implementation used raw
little-endian integers with no versioning or federation isolation guard.

This RFC is the canonical reference for all messages exchanged over the coordinator
UDS socket. RFC-0042 extended the protocol with link registration and a new minimal
data frame format; those additions are documented here as the normative definition.

---

## Decision

All messages exchanged over the coordinator UDS socket use a two-level encoding:

1. **Outer frame** (defined here): topic-length-prefixed framing, identical on both
   sides of the socket.
2. **Inner payload** (FlatBuffer table or raw binary struct, specified per-topic
   below).

The framing scheme and payload schemas here are the Single Source of Truth. Any code
that hand-rolls raw bytes on these topics without following the per-topic encoding
violates this RFC.

---

## Wire Format: Outer Frame

Every message — in both directions — is serialized as:

```
┌───────────────────────────────┐
│  topic_len  : u32 LE          │  4 bytes — byte length of the UTF-8 topic string
│  topic      : [u8; topic_len] │  UTF-8, no NUL terminator
│  payload_len: u32 LE          │  4 bytes — byte length of the payload
│  payload    : [u8; payload_len]│  encoding determined by topic (see below)
└───────────────────────────────┘
```

- All multi-byte integers in the frame header are **little-endian**.
- The topic is UTF-8 and must not exceed 4 GiB. In practice all topics are under
  256 bytes.
- The payload encoding is determined entirely by the topic string — no type tag in
  the outer frame.
- A zero-length payload is valid.

**Implementation references:**
- `hw/rust/backbone/transport-uds/src/lib.rs` — `write_framed()` (node side, sync)
- `tools/deterministic_coordinator/src/main.rs` — `uds_write_framed()` (coordinator
  side, async)

---

## Socket Path Convention

```
{run_dir}/{federation_id}/coordinator.sock
```

- `run_dir` defaults to `/run/virtmcu`; overridden by `VIRTMCU_RUN_DIR` or
  `--run-dir`.
- `federation_id` is the HLA federation name passed to `deterministic_coordinator`
  via `--federation-id`.
- Each coordinator instance owns exactly one socket file. Concurrent simulations
  must use distinct `federation_id` values.

---

## Protocol Version

```rust
pub const UDS_PROTO_VERSION: u32 = 2;
```

Defined in `hw/rust/common/virtmcu-wire/src/lib.rs`. Nodes embed this constant in
every `UdsRegistration` message. The coordinator asserts received `proto_version ==
UDS_PROTO_VERSION`; a mismatch is a **fatal `std::process::abort()`** (RFC-0022).

Version increments require a matching change to this RFC and a bump of
`UDS_PROTO_VERSION`. Backward compatibility is not required; simulations are always
compiled and run together. All schema, encoder, decoder, and coordinator changes must
land in a single commit (intra-commit lockstep).

---

## Topics and Payloads

### `sim/coord/register` — Node → Coordinator

**Direction**: node → coordinator  
**When**: first framed message sent by a node after `connect(2)`, before any other
topic  
**Payload**: `UdsRegistration` FlatBuffer root

```fbs
table UdsRegistration {
    node_id:       uint32;
    federation_id: string (required);
    proto_version: uint32;
}
```

**Coordinator behaviour:**
1. Parse with `virtmcu_wire::decode_uds_registration()`.
2. `std::process::abort()` if `proto_version != UDS_PROTO_VERSION`.
3. `std::process::abort()` if `federation_id != coordinator_federation_id`.
4. Record `connection → node_id` in the connection map. This mapping is the
   authoritative source of `src_node_id` for all subsequent messages on this
   connection — the message payload never carries `src_node_id`.
5. Emit `WorkerEvent::Register(node_id, write_half)` to the coordinator state
   machine.

**Node behaviour (encode):** call
`virtmcu_wire::encode_uds_registration(node_id, federation_id)`.

---

### `sim/coord/link/register` — Node → Coordinator

**Direction**: node → coordinator  
**When**: after `sim/coord/register`, once per link the peripheral participates in;
must complete for all `(node_id, link_name)` pairs before `sim/coord/start` is
issued  
**Payload**: `LinkRegistration` FlatBuffer root

```fbs
/// node_id is NOT in the payload — coordinator derives it from the connection map.
/// protocol is NOT in the payload — coordinator validates from topology.
table LinkRegistration {
    link_name: string (required);   /// must match a name in topology.links exactly
}
```

**Coordinator behaviour:**
1. Derive `node_id` from the connection map. If connection is unregistered →
   `std::process::abort()`.
2. Look up `link_name` in `link_ids` (built from topology at startup). If absent →
   `std::process::abort()`:
   `"FATAL: node {N} registered link '{name}' not in topology"`.
3. Assert `node_id ∈ rx_map[link_id]`. If absent → `std::process::abort()`:
   `"FATAL: node {N} not listed in topology participants for link '{name}'"`.
4. Mark `(node_id, link_id)` as registered.
5. Send `LinkAck { link_id, status: 0 }` on this node's write socket.

**Node behaviour (encode):** call
`virtmcu_wire::encode_link_registration(link_name)`.

---

### `sim/coord/link/ack` — Coordinator → Node

**Direction**: coordinator → node  
**When**: immediately after each `sim/coord/link/register` is processed  
**Payload**: `LinkAck` FlatBuffer root

```fbs
table LinkAck {
    link_id:   uint32;  /// coordinator-assigned; same value for all participants on the link
    status:    uint8;   /// 0 = OK; non-zero = FATAL
    error_msg: string;  /// populated only when status != 0
}
```

All participants of the same named link receive the same `link_id`. Link IDs are
assigned monotonically from 1; 0 is reserved (no link assigned).

**Node behaviour:** `std::process::abort()` if `status != 0`. Store `link_id`; pass
it to `reserve_link(link_id, …)` and `VtimeIngress::new_for_link(link_id, …)`.

---

### `sim/coord/start/{node_id}` — Coordinator → Node

**Direction**: coordinator → node  
**When**: (a) after all N nodes have completed link registration (pre-flight gate);
(b) after each subsequent PDES barrier release  
**Payload**: `UdsQuantumStart` FlatBuffer root

```fbs
table UdsQuantumStart {
    quantum:        uint64;
    vtime_limit_ns: uint64;
}
```

- `quantum`: 0-based quantum number being released.
- `vtime_limit_ns`: upper bound on `delivery_vtime_ns` for frames included in the
  DONE signal for this quantum. Currently always `u64::MAX`; reserved for future
  sub-quantum flow control.

**Coordinator behaviour (encode):**
`virtmcu_wire::encode_uds_quantum_start(quantum, u64::MAX)`.  
**Node behaviour (decode):**
`virtmcu_wire::decode_uds_quantum_start(bytes)`.

---

### `sim/coord/done/{node_id}/q/{quantum}` — Node → Coordinator

**Direction**: node → coordinator  
**When**: node has finished executing quantum `{quantum}`  
**Payload**: raw little-endian `u64` (8 bytes) — the quantum number

> This topic retains the raw u64 encoding; migration to a FlatBuffer is deferred to
> a future RFC.

---

### `sim/ch/{link_id}` — Node → Coordinator (Data TX)

**Direction**: node → coordinator  
**When**: peripheral has a protocol frame to transmit on link `link_id`  
**Payload**: minimal binary frame — no FlatBuffer

```
┌──────────────────────────┐
│ link_id    : u32 LE      │  4 bytes  — which link (validated against rx_map)
│ payload_len: u32 LE      │  4 bytes  — byte count of the protocol frame
│ payload    : [u8]        │  N bytes  — opaque protocol frame (CAN, Ethernet, etc.)
└──────────────────────────┘
```

The peripheral supplies nothing else. The coordinator derives:
- `src_node_id` — from the connection map (not the frame)
- `delivery_vtime_ns` — `quantum_vtime + delay_map[link_id]`
- `sequence_number` — per-connection receive counter (FIFO socket = deterministic)
- `dst_node_ids` — `rx_map[link_id]` minus `src_node_id`

**Coordinator routing rule:**
```rust
let src = connection_map[&conn];
let link_id = u32::from_le_bytes(frame[0..4]);
let payload = &frame[8..];
let delivery_vtime = quantum_vtime + delay_map[&link_id];
let seq = per_conn_counter[&conn].fetch_add(1, Relaxed);
for &dst in rx_map[&link_id].iter().filter(|&&n| n != src) {
    deliver_to(&sockets[&dst], link_id, src, delivery_vtime, seq, payload);
}
```

`std::process::abort()` if `link_id` is not in `rx_map`. No string matching. No
protocol detection. No silent fallback.

**Rationale**: a peripheral is a hardware model — it puts bytes on a link. The
medium (simulated by the coordinator) determines who receives them and when. See
RFC-0042 §"Motivation" for the full design reasoning.

---

### `sim/ch/{link_id}` — Coordinator → Node (Data RX)

**Direction**: coordinator → node  
**When**: after the PDES barrier for quantum Q, for each frame destined for a node  
**Payload**: binary delivery frame — no FlatBuffer

```
┌──────────────────────────────┐
│ link_id          : u32 LE    │  4 bytes  — receiving VtimeIngress dispatches on this
│ src_node_id      : u32 LE    │  4 bytes  — coordinator-assigned; for pcap/logging
│ delivery_vtime_ns: u64 LE    │  8 bytes  — coordinator-assigned; VtimeIngress timer
│ sequence_number  : u64 LE    │  8 bytes  — coordinator-assigned; PDES tie-breaking
│ payload_len      : u32 LE    │  4 bytes
│ payload          : [u8]      │  N bytes  — identical bytes the sender put in
└──────────────────────────────┘
```

`src_node_id` is coordinator-assigned and cannot be spoofed by the sender. Firmware
must not use it for application-level routing — protocol-level addressing (MAC
filter, CAN acceptance mask) is the firmware's responsibility, as on real hardware.

`VtimeIngress::new_for_link(link_id, …)` subscribes to `"sim/ch/{link_id}"` and
parses this header before invoking the user-supplied decode closure.

---

## Deprecated Topics (Stage 1: still accepted; Stage 3: removed)

The following topics are deprecated by RFC-0042 and will be removed once all
peripherals have migrated to `sim/ch/{link_id}`.

### `sim/chardev/{node_id}/tx` — Node → Coordinator *(deprecated)*

**Replaced by**: `sim/ch/{link_id}`  
**Payload**: 24-byte `ZenohFrameHeader` struct followed by raw network bytes

```fbs
/// Deprecated — do not use in new peripheral code
struct ZenohFrameHeader {
    delivery_vtime_ns: uint64;
    sequence_number:   uint64;
    size:              uint32;
}
```

### `sim/coord/{node_id}/rx` — Coordinator → Node *(deprecated)*

**Replaced by**: `sim/ch/{link_id}`  
**Payload**: 20-byte binary header followed by raw network bytes

```
│  src_node_id       : u32 LE  │
│  dst_node_id       : u32 LE  │  ← eliminated in new format
│  delivery_vtime_ns : u64 LE  │
│  sequence_number   : u64 LE  │
│  <raw network bytes>         │
```

`dst_node_id` is absent from the new delivery format. The coordinator routes
exclusively by topology; the peripheral never specifies a destination.

---

## Pre-Flight Sequencing

The coordinator enforces a strict startup order. No data frames are processed until
the pre-flight gate clears.

```
Node 0 peripheral         Coordinator              Node 1 peripheral
       │                       │                          │
       │──sim/coord/register──►│◄──sim/coord/register─────│
       │                       │  records connection→node_id
       │                       │  (barrier: wait N nodes)
       │                       │
       │──sim/coord/link/register──►│◄──sim/coord/link/register──│
       │  {link_name="ref_bus"}│    {link_name="ref_bus"}
       │                       │  validates topology; assigns link_id=1
       │◄──sim/coord/link/ack──│──sim/coord/link/ack──────────►│
       │  {link_id=1,status=0} │   {link_id=1,status=0}
       │                       │
       │                       │  (pre-flight: all (node,link) pairs registered
       │                       │   30-second timeout; abort with missing list)
       │◄──sim/coord/start/0── │──sim/coord/start/1───────────►│
       │                       │
       │──sim/ch/1─────────────►│  [link_id=1][len=4][payload]
       │                       │  src=0 (connection); vtime=quantum+delay[1]
       │                       │  routes to rx_map[1]=[0,1]-{0} → node 1
       │──sim/coord/done/0/q/0►│◄──sim/coord/done/1/q/0────────│
       │                       │  (PDES barrier)
       │                       │──sim/ch/1────────────────────►│
       │                       │  [1][src=0][vtime][seq][len][payload]
       │◄──sim/coord/start/0── │──sim/coord/start/1───────────►│
```

---

## Proto Version Negotiation Rules

1. **No handshake negotiation**: the coordinator asserts one exact version.
2. **Hard failure on mismatch**: `std::process::abort()` with a descriptive message.
3. **Version bump policy**: increment `UDS_PROTO_VERSION` whenever any topic payload
   schema changes in a backward-incompatible way. Re-generate `core_generated.rs`
   via `cargo xtask flatc` before committing.
4. **Intra-commit lockstep**: all changes to the wire protocol (schema, encoder,
   decoder, coordinator parser, test clients) must land in a single commit.

---

## FlatBuffers Schema Location

```
hw/rust/common/virtmcu-wire/src/core.fbs          # source of truth
hw/rust/common/virtmcu-wire/src/core_generated.rs # generated — do not edit
```

Regenerate with:
```bash
cargo xtask flatc
```

---

## Consequences

- **Determinism**: federation ID and proto_version checks prevent cross-simulation
  socket confusion in multi-instance test environments.
- **Routing authority**: the coordinator is the sole routing and timestamping
  authority. Peripherals supply payload only; they never specify destination,
  delivery time, or sequence number.
- **Schema evolution**: adding a new optional FlatBuffer field does not require a
  version bump. Removing or changing a field type does.
- **Binary data path**: `sim/ch/{link_id}` uses a fixed binary layout, not
  FlatBuffers.
