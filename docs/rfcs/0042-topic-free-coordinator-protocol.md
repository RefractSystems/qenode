# RFC-0042: Topic-Free Coordinator Protocol — Named Links and Hub-Routed Channels

## Status

Draft (Revised — see Self-Critique Log for design evolution)

## Summary

Replace the coordinator's substring-matched topic routing with a **hub-and-spoke
model** backed by **named links** in the topology YAML. Each QEMU node has exactly
one UDS socket to the coordinator. Each link in the topology is assigned a single
`u32 link_id`. Nodes send minimal frames tagged with `link_id`; the coordinator
derives all routing metadata from connection identity and topology, then fans out to
all other participants of that link. The coordinator is
the sole routing and timestamping authority.

## Self-Critique Log (Design Iterations)

| Iteration | Problem found | Resolution |
|---|---|---|
| Draft 1 | `link_index` is YAML-order-dependent; reorder = silent breakage | Mandatory `name:` on every link |
| Draft 1 | `frame_type: u8` sentinel conflicts with RFC-0033 `topic_len: u32 LE` | Data frame topic `"sim/ch/{link_id}"` — RFC-0033-framed, no ambiguity |
| Draft 1 | Pre-flight barrier has no timeout | 30 s timeout; panic names missing `(node_id, link_name)` |
| Draft 1 | Zenoh path called "out of scope" not "deprecated" | Explicitly deprecated; follow-on RFC scoped |
| Draft 1 | Multi-cast channel assignment underspecified | Hub model: coordinator routes; no per-node channel math |
| Draft 1 | `role` field missing from FlatBuffer | Added `LinkRole` enum |
| Draft 1 | `link_name` source was Unresolved Question | `yaml2qemu` injection is a Stage 1 deliverable |
| Draft 1 | `VtimeIngress` migration implicit | `new_for_link(link_id, …)` scoped to Stage 2 |
| Draft 1 | `reserve()` removal timeline vague | Deprecated Stage 1; removed Stage 3 only after Zenoh migrated |
| Draft 2 | Separate TX/RX channel pools multiply IDs by 2N per link | **Eliminated** — single `link_id` per link shared by all participants |
| Draft 2 | `rx_map` mapped `tx_channel_id → [rx_channel_ids]` — O(N·M) IDs | `rx_map: HashMap<link_id, Vec<node_id>>` — O(M) entries |
| Draft 2 | Multi-cast required `LinkAck` to return a list of `rx_channel_ids` | `LinkAck` returns one scalar `link_id`; coordinator handles fan-out |
| Draft 3 | `delivery_vtime_ns` in TX frame forces peripheral to compute delivery time | **Eliminated from TX wire format** — coordinator assigns `quantum_vtime + topology.propagation_delay(link_id)`; peripheral models hardware, not scheduling |
| Draft 3 | `src_node_id` in TX frame is redundant with connection identity | **Eliminated from TX wire format** — coordinator derives from the socket that delivered the frame; prevents node-ID spoofing |
| Draft 3 | `register_link(node_id, link_name, protocol, role)` leaks node_id and protocol into the peripheral API | `node_id` derived from connection; `protocol` validated by coordinator from topology; `role` removed (coordinator routes all participants symmetrically) — API becomes `register_link(link_name: &str) -> Result<u32>` |
| Draft 3 | `sequence_number` in TX frame requires peripheral to maintain a counter | **Eliminated from TX frame** — coordinator assigns sequence from FIFO socket receive order, which is deterministic on UDS; peripheral has no ordering responsibility |

## Motivation

### The Root Bug Pattern

RFC-0024 mandated assertion-based routing but the data plane still routes by
substring-matching topic strings:

```rust
let proto = if topic.contains("eth") { Protocol::Ethernet }
            else if topic.contains("chardev") || topic.contains("reference_bus") {
                Protocol::ReferenceLink
            }
            else { Protocol::Ethernet }; // ← silent wrong-protocol fallback
```

The concrete failure: `worlds/reference_ping_pong.yml` used `topic: 'reference_bus'`
but the coordinator only subscribed to `sim/chardev/*/tx`. Node 1 hung forever. The
patch added another alias. Every new peripheral or renamed topic repeats the cycle.

### The Correct Mental Model

The coordinator is the hub. Every node connects with **one socket**. The topology
YAML declares who talks to whom. The coordinator should be the **sole routing and
timestamping authority**. A peripheral is a hardware model: it puts bytes on a link.
The medium — simulated by the coordinator — determines who receives them, when.

This matches real hardware exactly:
- A CAN peripheral puts a frame on the bus. It does not address a destination node.
- An Ethernet NIC puts a frame on the wire. It does not schedule delivery time.
- An 802.15.4 radio transmits. The channel propagation model handles timing.

### SOTA Comparison

| Framework | Connections | Routing authority |
|---|---|---|
| SystemC TLM 2.0 | Port-to-port (elaboration-time) | Compile-time socket binding |
| AUTOSAR COM | Each ECU → COM middleware | COM middleware routes by signal ID |
| CAN bus | Each node → bus (shared medium) | Bus arbitration; receiver filters by message ID |
| VirtMCU (current) | Node → Coordinator (one socket) | Coordinator guesses protocol by topic substring |
| **VirtMCU (this RFC)** | **Node → Coordinator (one socket)** | **Coordinator owns all routing, scheduling, and fan-out; peripheral sends payload only** |

## Detailed Design

### Concept: Named Links and Hub Routing

```
                    ┌─────────────────────┐
   Node 0 ──────── │                     │ ──────── Node 1
   Node 1 ──────── │   Coordinator       │ ──────── Node 0
   Node 2 ──────── │   (topology YAML)   │ ──────── Node 0
                   │                     │ ──────── Node 1
                   └─────────────────────┘

   Each node has ONE socket.
   Each link has ONE link_id.
   Coordinator routes by (connection identity → src_node_id,
                          link_id → topology → destination node_ids,
                          quantum_vtime + propagation_delay → delivery_vtime_ns).
```

Every link in the topology YAML has a mandatory `name:` field:

```yaml
topology:
  links:
    - name: ref_bus        # mandatory; stable regardless of ordering
      type: reference-link
      nodes: ['0', '1']
    - name: vehicle_can    # mandatory
      type: can-fd
      nodes: ['0', '1', '2']
      propagation_delay_ns: 500   # optional; defaults to 0
```

Link names are lowercase, hyphens allowed, no slashes. `yaml2qemu` hard-errors
on any link missing `name:` or on duplicate names within a file.

---

### Phase 0 — yaml2qemu: Inject `link-name` QOM Property

`yaml2qemu` injects a `link-name` QOM property into each peripheral that
participates in a topology link.

```yaml
- name: reference_peripheral
  type: reference-peripheral
  properties:
    link-name: ref_bus   # ← injected by yaml2qemu; 'topic' is now ILLEGAL
```

```rust
// In the peripheral QOM struct
#[qom_property(name = "link-name")]
pub link_name: virtmcu_qom::qom::QomString;
```

The peripheral stores `link_name`. It never constructs a topic string and never
touches `node_id` in any transport call.

---

### Phase 1 — Coordinator Startup: Link ID Assignment

At startup the coordinator builds two maps from the topology YAML:

```rust
/// link_name → link_id  (monotonically assigned from 1; 0 = reserved/unassigned)
link_ids: HashMap<String, u32>

/// link_id → Vec<node_id>  (all participants; sender excluded at delivery time)
rx_map: HashMap<u32, Vec<u32>>

/// link_id → propagation_delay_ns
delay_map: HashMap<u32, u64>
```

**Example:**

```
link_ids:  "ref_bus"     → 1
           "vehicle_can" → 2

rx_map:    1 → [0, 1]
           2 → [0, 1, 2]

delay_map: 1 → 0
           2 → 500
```

The coordinator also builds a `connection → node_id` map from the
`sim/coord/register` handshake. This is the authoritative source of `src_node_id`
for every subsequent message on that connection.

---

### Phase 2 — Link Registration Protocol

Two new FlatBuffer tables in `hw/rust/common/virtmcu-wire/src/core.fbs`.
`UDS_PROTO_VERSION` is incremented.

```fbs
/// Node → Coordinator, topic: "sim/coord/link/register"
/// node_id is NOT included — coordinator derives it from the connection.
/// protocol is NOT included — coordinator validates from topology.
table LinkRegistration {
  link_name: string (required);   /// must match a name in topology.links
}

/// Coordinator → Node, topic: "sim/coord/link/ack"
table LinkAck {
  link_id:   uint32;  /// coordinator-assigned; same value for all participants
  status:    uint8;   /// 0 = OK; non-zero = FATAL
  error_msg: string;  /// populated only when status != 0
}
```

**Coordinator behaviour on receiving `LinkRegistration`:**

1. Derive `node_id` from the connection map. If connection is unregistered →
   **immediate `std::process::abort()`** (invariant: `sim/coord/register` always
   precedes data).
2. Look up `link_name` in `link_ids`. If absent → **immediate abort**:
   `"FATAL: node {N} registered link '{name}' not in topology"`.
3. Assert `node_id ∈ rx_map[link_id]`. If absent → **immediate abort**:
   `"FATAL: node {N} not listed in topology participants for link '{name}'"`.
4. Mark `(node_id, link_id)` as registered.
5. Send `LinkAck { link_id, status: 0 }` on the same connection.

All participants of the same link receive the same `link_id`. No per-node channel
arithmetic.

---

### Phase 3 — Pre-Flight Barrier with Timeout

After all N nodes send `sim/coord/register`, the coordinator enters the **link
registration phase**. It waits for every `(node_id, link_name)` pair in the
topology to send a matching `LinkRegistration`.

**Timeout**: 30 seconds (configurable via `--link-registration-timeout-secs`).
On expiry the coordinator **aborts** with the full list of missing registrations:

```
FATAL: link registration timeout after 30s.
  Missing:
    node=0  link='ref_bus'
    node=2  link='vehicle_can'
  Hints:
    (1) Does 'link-name' QOM property match the topology link name exactly?
    (2) Was peripheral realize() called before the timeout?
    (3) Did yaml2qemu inject 'link-name' for this peripheral?
```

Only after all registrations are received does the coordinator issue
`sim/coord/start` to unblock each node's VCPUs.

---

### Phase 4 — Data Frame Format (Peripheral → Coordinator)

```
Topic (RFC-0033 framed): "sim/ch/{link_id}"

Payload:
┌──────────────────────────┐
│ link_id    : u32 LE      │  4 bytes  — which channel
│ payload_len: u32 LE      │  4 bytes  — byte count of the protocol frame
│ payload    : [u8]        │  N bytes  — opaque protocol frame (CAN, Ethernet, etc.)
└──────────────────────────┘
```

That is the complete TX format. The peripheral supplies nothing else.

The coordinator derives at receipt:
- `src_node_id` — from the connection map (not the frame)
- `delivery_vtime_ns` — `quantum_vtime + delay_map[link_id]`
- `sequence_number` — coordinator's per-connection receive counter (FIFO socket = deterministic)
- `dst_node_ids` — `rx_map[link_id]` minus `src_node_id`

**Routing rule:**

```rust
// Everything the coordinator needs — zero string matching, zero protocol detection.
let src_node_id: u32 = connection_map[&conn];
let link_id: u32 = read_u32_le(&frame);
let payload: &[u8] = &frame[8..];

let delivery_vtime = quantum_vtime + delay_map[&link_id];
let seq = per_connection_counters[&conn].fetch_add(1, Ordering::Relaxed);

for &dst in rx_map[&link_id].iter().filter(|&&n| n != src_node_id) {
    deliver_to(&sockets[&dst], link_id, src_node_id, delivery_vtime, seq, payload);
}
```

No `if topic.contains(…)`. No `else { Protocol::Ethernet }`. No FlatBuffer decode.
One rule, one abort on unknown link_id, zero silent fallbacks.

---

### Phase 4b — Delivery Frame Format (Coordinator → Node)

The coordinator delivers using the same RFC-0033 topic framing on the destination
node's socket. The delivery payload adds coordinator-assigned metadata:

```
Topic: "sim/ch/{link_id}"

Payload:
┌──────────────────────────────┐
│ link_id          : u32 LE    │  4 bytes  — receiving VtimeIngress dispatches on this
│ src_node_id      : u32 LE    │  4 bytes  — for pcap/logging; opaque to firmware
│ delivery_vtime_ns: u64 LE    │  8 bytes  — coordinator-assigned; VtimeIngress timer
│ sequence_number  : u64 LE    │  8 bytes  — coordinator-assigned; PDES tie-breaking
│ payload_len      : u32 LE    │  4 bytes
│ payload          : [u8]      │  N bytes  — same bytes the sender put in
└──────────────────────────────┘
```

`src_node_id` is included for logging and pcap. It is coordinator-assigned and
cannot be spoofed by the sender. Firmware must not use it for routing decisions —
protocol-level addressing (MAC filter, CAN acceptance mask) is the firmware's
responsibility, as on real hardware.

---

### Phase 5 — VtimeIngress API

`VtimeIngress::new_for_link(link_id, …)` subscribes to `"sim/ch/{link_id}"` on the
transport and parses the delivery frame format defined in Phase 4b. It schedules a
`ClosureTimer` at `delivery_vtime_ns` for each received packet.

The deprecated `VtimeIngress::new(topic, …)` is removed in Stage 3.

---

### Phase 6 — DataTransport Trait

```rust
pub trait DataTransport: Send + Sync {
    /// Register a link by name; returns the coordinator-assigned link_id.
    /// node_id and protocol are NOT parameters — the coordinator derives both.
    fn register_link(
        &self,
        link_name: &str,
    ) -> Result<u32, TransportError>;

    /// Reserve a zero-copy buffer for a TX frame on this link.
    /// The transport frames it as [link_id: u32][payload_len: u32][payload: bytes].
    fn reserve_link(
        &self,
        link_id: u32,
        len: usize,
    ) -> Result<Reservation<'_>, TransportError>;

    /// Deprecated: reserve by topic string. Removed in Stage 3.
    #[deprecated(since = "0.3.0", note = "use reserve_link()")]
    fn reserve(&self, topic: &str, len: usize) -> Result<Reservation<'_>, TransportError>;
}
```

`node_id` is removed from `register_link`. The `node-id` QOM property may remain on
peripherals for logging, but it is never passed to any transport call. A peripheral
that does not know its simulation node ID is still a fully correct peripheral.

---

### Zenoh Path: Deprecated, Not Extended

RFC-0019 establishes UDS as the default single-host transport. The Zenoh data path
is **deprecated for simulation data frames**. This RFC targets UDS only. A follow-on
RFC will extend the link registration protocol to Zenoh using a coordinator-published
channel map at federation startup.

---

### Migration Stages

| Stage | Coordinator | Peripherals | `DataTransport` | Gate |
|---|---|---|---|---|
| **1** | Handle `sim/coord/link/register`; build `rx_map`/`delay_map` from topology; route `sim/ch/{link_id}` frames via minimal binary format; `yaml2qemu` injects `link-name` | No change | `register_link(link_name)` + `reserve_link(link_id)` added; stubs replaced with real implementations; `reserve()` deprecated | `make test-check` green; `test_reference_ping_pong_*` pass |
| **2** | Legacy topic path (`sim/chardev/…`, `sim/eth/…`, etc.) still accepted in parallel | Migrate one peripheral at a time; `VtimeIngress::new_for_link()`; no new `topic` QOM properties | `reserve()` `#[deprecated]`; `#![allow(deprecated)]` stubs removed per peripheral | All integration tests green |
| **3** | Delete substring routing, wildcard constants, topic template functions | `topic` property removed from all peripheral structs and world YAMLs | `reserve()` removed; `ZenohFrameHeader`/`encode_frame`/`decode_frame` deleted | `make ci-full` green |

---

### What Is Removed (Stage 3 Complete)

| Removed | Replaced by |
|---|---|
| `CoordMessage` FlatBuffer table (data path) | `[link_id: u32][payload_len: u32][payload: bytes]` — 3-field binary frame |
| `dst_node_id` in any message | Topology lookup: `rx_map[link_id] - {src}` |
| `delivery_vtime_ns` in TX frame | Coordinator assigns: `quantum_vtime + delay_map[link_id]` |
| `src_node_id` in TX frame | Coordinator derives from connection identity |
| `sequence_number` in TX frame | Coordinator assigns from per-connection receive counter |
| `node_id` parameter in `register_link` | Coordinator derives from connection |
| `protocol` parameter in `register_link` | Coordinator validates from topology |
| `role` parameter in `register_link` | Coordinator routes all participants symmetrically |
| `topic` QOM property on all peripherals | `link-name` (injected by `yaml2qemu`) |
| `DataTransport::reserve(topic, size)` | `DataTransport::reserve_link(link_id, size)` |
| `VtimeIngress::new(topic, …)` | `VtimeIngress::new_for_link(link_id, …)` |
| `encode_coord_message` / `decode_coord_message` | No equivalent — data path is now opaque bytes |
| `encode_frame` / `decode_frame` | No equivalent — peripheral puts raw protocol frame in payload |
| `ZenohFrameHeader` | No equivalent |
| All wildcard constants (`CHARDEV_TX_WILDCARD`, …) | No wildcards — link_id is exact |
| All topic template functions (`chardev_tx`, …) | No topic templates |
| `topic.contains(…)` chain with silent Ethernet fallback | `rx_map.get(&link_id)` — abort on miss |
| Per-node per-protocol Zenoh subscriptions | Zero Zenoh data subscriptions in UDS mode |
| Nested FlatBuffer decode fallback chain | One binary frame read, zero fallbacks |

---

### Updated Sequencing Diagram

```
Node 0 peripheral              Coordinator              Node 1 peripheral
       │                            │                          │
       ├── sim/coord/register ───-─►│◄── sim/coord/register ─-─┤
       │                            │  builds: connection→node_id map
       │                            │  (barrier: wait N nodes) │
       │                            │                          │
       ├── sim/coord/link/register-►│◄─sim/coord/link/register-┤
       │   {link_name="ref_bus"}    │   {link_name="ref_bus"}  │
       │                            │  derives: src=0 from connection
       │                            │  validates: "ref_bus" ∈ topology
       │                            │  assigns:   link_id=1    │
       │◄──── sim/coord/link/ack ──┤├─-─ sim/coord/link/ack ──►│
       │    {link_id=1, status=0}   │    {link_id=1, status=0} │
       │                            │                          │
       │                            │  (pre-flight: all pairs registered)
       │◄────── sim/coord/start ───┤├─── sim/coord/start ─────►│
       │                            │                          │
       ├── sim/ch/1 ───────────-───►│   [link_id=1][len=4][0x50494e47]
       │                            │  derives: src=0 (connection)
       │                            │  computes: delivery_vtime=quantum+delay[1]
       │                            │  routes:   rx_map[1]=[0,1] → deliver to 1
       ├── sim/coord/done/q/0 ──-──►│◄── sim/coord/done/q/1 ─-─┤
       │                            │  (PDES barrier)          │
       │                            │├── sim/ch/1 ────────────►│
       │                            │  [link_id=1][src=0][vtime][seq][len][payload]
       │◄────── sim/coord/start ───┤├─── sim/coord/start ─────►│
```

## Drawbacks

**Wire protocol flag-day at Stage 1.** `UDS_PROTO_VERSION` is incremented; all
coordinator, transport, and schema changes land in one commit (RFC-0033 policy).

**`link-name` requires YAML discipline.** All topology files must add `name:` to
every link. `yaml2qemu migrate-link-names` auto-generates names from
`{type}-{index}` for existing files.

**Zenoh path remains on deprecated topic routing.** Accepted temporary divergence;
a follow-on RFC closes the gap.

**`delivery_vtime_ns` precision is quantum-granular.** All frames sent within one
quantum receive the same base delivery time. Sub-quantum precision (PWM, bit timing)
requires `slaved-icount` mode, where the coordinator may opt in to a finer-grained
timestamp source. This is an advanced opt-in; behavioral simulation is unaffected.

## Alternatives

**Alt B: Use protocol headers (Ethernet MAC, CAN arbitration ID) as routing keys.**
Works for point-to-point protocols; fails for bus protocols (CAN, FlexRay, LIN)
which are inherently broadcast with no destination address. The coordinator would
need to parse every protocol's header format — an unbounded maintenance surface.
The `link_id` approach is protocol-agnostic. Rejected.

**Alt C: Per-link UDS socket.**
Each link gets its own FD. No `link_id` needed. Bounded by OS open-FD limit;
doubles coordinator epoll complexity. Rejected.

**Alt D: Keep topic strings, validate at startup.**
Pre-compute all valid topics; panic on unknown. Keeps string parsing in the hot
path. Rejected.

**Cost of doing nothing:** topic aliases accumulate per release; the silent
`Protocol::Ethernet` fallback stays; every new peripheral author hits the
`chardev`/`reference_bus` class of bug; the nested FlatBuffer decode fallback chain
grows with every new protocol.

## Prior Art

- **AUTOSAR COM**: signal routing resolved at compile time; integer signal IDs in
  generated code. No runtime string lookup.
- **CAN bus**: each frame carries a message ID; the bus delivers to all listeners;
  filtering is the receiver's responsibility. Direct analogue to link_id fan-out.
- **gem5**: port IDs assigned at `connectAllPorts()`; routing by integer.
- **RFC-0033 (this codebase)**: `UdsRegistration` proves typed startup-time
  negotiation is already feasible.

## New FlatBuffers Required

Two tables in `hw/rust/common/virtmcu-wire/src/core.fbs`:

1. **`LinkRegistration`** — node → coordinator at realize time (`link_name` only)
2. **`LinkAck`** — coordinator → node, carries `link_id`

`ZenohFrameHeader`, `CoordDoneReq`, and all existing tables are
**deleted** in Stage 3. The data path no longer uses FlatBuffers.

The quantum barrier signal (`sim/coord/done`) is a bare `u64` quantum number.

Regenerate after schema change:
```bash
cargo xtask flatc
```

## Unresolved Questions

- **`yaml2qemu migrate-link-names`**: auto-generation algorithm is
  `{link_type}-{zero_padded_index}`. Whether names require human review before merge
  is a project-policy question resolved during Stage 1 implementation.

- **Zenoh distributed channel negotiation**: the follow-on RFC must specify how
  link IDs are agreed upon without a shared UDS socket. Leading candidate:
  coordinator publishes the full `link_id` map on a well-known Zenoh key at
  federation startup.

- **Sub-quantum delivery precision**: whether `slaved-icount` mode should allow
  the peripheral to pass a within-quantum offset is deferred to the follow-on RFC
  that addresses PWM and bit-timing simulation.
