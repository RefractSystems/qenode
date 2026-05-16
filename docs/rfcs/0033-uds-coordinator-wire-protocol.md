# RFC-0033: UDS Coordinator Wire Protocol

## Status
Accepted

## Context

RFC-0019 established Unix Domain Sockets (UDS) as the default single-host transport for VirtMCU simulations, but left the on-the-wire framing, registration handshake, and quantum-start signalling underspecified. The implementation used raw little-endian integers (`u32` for node ID, `u64` for quantum number), with no versioning, no federation isolation guard, and no SSoT schema.

Task 11.3b (PLAN.md) completed the migration: all coordinator ↔ node messages are now FlatBuffers, and this RFC is the canonical reference.

## Decision

All messages exchanged over the coordinator UDS socket use a two-level encoding:

1. **Outer frame** (defined here): topic-length-prefixed framing, identical on both sides of the socket.
2. **Inner payload** (FlatBuffer table or raw byte struct, specified per-topic below).

The framing scheme and FlatBuffer schemas are the Single Source of Truth. Any code that hand-rolls raw bytes on these topics violates this RFC.

---

## Wire Format: Outer Frame

Every message — in both directions — is serialized as:

```
┌───────────────────────────────┐
│  topic_len  : u32 LE          │  4 bytes — byte length of the UTF-8 topic string
│  topic      : [u8; topic_len] │  UTF-8, no NUL terminator
│  payload_len: u32 LE          │  4 bytes — byte length of the payload
│  payload    : [u8; payload_len]│  FlatBuffer root or raw struct (see per-topic below)
└───────────────────────────────┘
```

- All multi-byte integers in the frame header are **little-endian**.
- The topic is UTF-8 and must not exceed 4 GiB (u32 length). In practice, all topics in this protocol are well under 256 bytes.
- The payload encoding is determined entirely by the topic string — there is no type tag in the outer frame.
- A zero-length payload is valid; a `payload_len = 0` frame carries no bytes after the length field.

**Implementation references:**
- `hw/rust/backbone/transport-unix/src/lib.rs` — `write_framed()` (node side, sync)
- `tools/deterministic_coordinator/src/main.rs` — `uds_write_framed()` (coordinator side, async)

---

## Socket Path Convention

```
{run_dir}/{federation_id}/coordinator.sock
```

- `run_dir` defaults to `/run/virtmcu` and is overridden by the `VIRTMCU_RUN_DIR` environment variable or the `--run-dir` CLI flag passed to `deterministic_coordinator`.
- `federation_id` is the HLA federation name injected at coordinator startup via `--federation-id`.
- Each coordinator instance owns exactly one socket file. Multiple concurrent simulations must use distinct `federation_id` values.

---

## Protocol Version

```rust
pub const UDS_PROTO_VERSION: u32 = 1;
```

Defined in `hw/rust/common/virtmcu-api/src/lib.rs`. Nodes embed this constant in every `UdsRegistration` message. The coordinator asserts that the received `proto_version` equals `UDS_PROTO_VERSION` at startup; a mismatch is a **fatal panic** (Fail Loudly, RFC-0022).

Version increments require a matching change to this RFC and a bump of `UDS_PROTO_VERSION`. Backward compatibility is not required; simulations are always compiled and run together.

---

## Topics and Payloads

### `sim/coord/register` — Node → Coordinator

**Direction**: node → coordinator  
**When**: first framed message sent by a node after `connect(2)`, before any other topic  
**Payload**: `UdsRegistration` FlatBuffer root

```fbs
/// hw/rust/common/virtmcu-api/src/core.fbs
table UdsRegistration {
    node_id:       uint32;
    federation_id: string (required);
    proto_version: uint32;
}
```

**Coordinator behaviour:**
1. Parse with `virtmcu_api::decode_uds_registration()`.
2. Assert `proto_version == UDS_PROTO_VERSION` — panics on mismatch.
3. Assert `federation_id == coordinator_federation_id` — panics on mismatch (prevents cross-federation socket confusion).
4. Emit `WorkerEvent::Register(node_id, write_half)` to the coordinator state machine.

**Node behaviour (encode):** call `virtmcu_api::encode_uds_registration(node_id, federation_id)`.

---

### `sim/coord/start/{node_id}` — Coordinator → Node

**Direction**: coordinator → node  
**When**: after all nodes have submitted their DONE signal for quantum Q; coordinator sends this to unblock node `{node_id}` for quantum Q+1  
**Payload**: `UdsQuantumStart` FlatBuffer root

```fbs
/// hw/rust/common/virtmcu-api/src/core.fbs
table UdsQuantumStart {
    quantum:        uint64;
    vtime_limit_ns: uint64;
}
```

- `quantum` is the 0-based quantum number the node is being released to execute.
- `vtime_limit_ns` is the upper bound on `delivery_vtime_ns` for any frame the node may include in its DONE signal for this quantum. Frames with `delivery_vtime_ns > vtime_limit_ns` must be deferred to a later quantum. Currently always `u64::MAX` (no bound enforced), reserved for future sub-quantum flow control.

**Coordinator behaviour (encode):** call `virtmcu_api::encode_uds_quantum_start(quantum, u64::MAX)`.  
**Node behaviour (decode):** call `virtmcu_api::decode_uds_quantum_start(bytes)`.

---

### `sim/coord/done/{node_id}/q/{quantum}` — Node → Coordinator

**Direction**: node → coordinator  
**When**: node has finished executing quantum `{quantum}`  
**Payload**: raw little-endian `u64` (8 bytes) — the quantum number, redundant with the topic but included for sanity checking

> **Note**: This topic predates the FlatBuffer migration and retains the raw u64 encoding for backward compatibility with the PDES barrier logic. A future RFC may migrate it to `CoordDoneReq` FlatBuffer.

---

### `sim/chardev/{node_id}/tx` — Node → Coordinator

**Direction**: node → coordinator  
**When**: node has a network frame to transmit  
**Payload**: 24-byte `ZenohFrameHeader` struct followed by raw network bytes

```fbs
/// hw/rust/common/virtmcu-api/src/core.fbs
struct ZenohFrameHeader {
    delivery_vtime_ns: uint64;
    sequence_number:   uint64;
    size:              uint32;
}
```

The coordinator holds these frames until the PDES barrier clears, then delivers them in canonical order `(delivery_vtime_ns, source_node_id, sequence_number)`.

---

### `sim/coord/{node_id}/rx` — Coordinator → Node

**Direction**: coordinator → node  
**When**: after the PDES barrier for quantum Q, for each frame destined for `node_id`  
**Payload**: 20-byte delivery header followed by raw network bytes

```
┌─────────────────────────────────┐
│  src_node_id  : u32 LE          │
│  dst_node_id  : u32 LE          │
│  delivery_vtime_ns : u64 LE     │
│  sequence_number   : u64 LE     │
│  <raw network bytes>            │
└─────────────────────────────────┘
```

> **Note**: This header is coordinator-internal and not a FlatBuffer. A future RFC may unify it under `CoordMessage`.

---

## Sequencing Diagram

```
Node 0                   Coordinator              Node 1
  │                          │                      │
  │──sim/coord/register──►   │  ◄──sim/coord/register──│
  │                          │  (waits for all N nodes) │
  │                          │                      │
  │──sim/chardev/0/tx──►     │                      │
  │──sim/coord/done/0/q/0──► │  ◄──sim/coord/done/1/q/0─│
  │                          │  (PDES barrier releases)  │
  │  ◄──sim/coord/0/rx──     │                      │
  │                          │  ──sim/coord/1/rx──► │
  │  ◄──sim/coord/start/0──  │  ──sim/coord/start/1─►│
  │  (quantum 1 released)    │  (quantum 1 released) │
```

---

## Proto Version Negotiation Rules

1. **No handshake negotiation**: the coordinator does not offer a capabilities list. It asserts one exact version.
2. **Hard failure on mismatch**: a `proto_version` in `UdsRegistration` that differs from `UDS_PROTO_VERSION` causes an immediate coordinator panic with a descriptive message. This prevents silent misparse of struct fields.
3. **Version bump policy**: increment `UDS_PROTO_VERSION` whenever any topic payload schema changes in a backward-incompatible way. Re-generate `core_generated.rs` via `cargo xtask flatc` before committing.
4. **Intra-commit lockstep**: all changes to the wire protocol (schema, encoder, decoder, coordinator parser, test clients) must land in a single commit. Partial migrations are explicitly forbidden.

---

## FlatBuffers Schema Location

```
hw/rust/common/virtmcu-api/src/core.fbs        # source of truth
hw/rust/common/virtmcu-api/src/core_generated.rs # generated — do not edit
```

Regenerate with:
```bash
cargo xtask flatc
```

---

## Consequences

- **Determinism**: federation ID and proto_version checks prevent cross-simulation socket confusion in multi-instance test environments.
- **Schema evolution**: adding a new optional field to a FlatBuffer table does not require a version bump (flatbuffers forward/backward compatibility). Removing or changing a field type does.
- **Future extensibility**: `vtime_limit_ns` in `UdsQuantumStart` is reserved for sub-quantum flow control (not yet implemented).
- **Raw u64 DONE signal**: retained as-is; migration is deferred to a dedicated RFC.
