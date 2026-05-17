# RFC-0042: Topic-Free Coordinator Protocol — Named Links and Hub-Routed Channels

## Status

Draft

## Summary

Replace the coordinator's substring-matched topic routing with a **hub-and-spoke
model** backed by **named links** in the topology YAML. Each QEMU node has exactly
one UDS socket to the coordinator (already the case). Each link in the topology is
assigned a single `u32 link_id`. Nodes send frames tagged with `link_id`; the
coordinator routes to all other participants of that link. Connections grow O(N),
link IDs grow O(M), and the coordinator is the sole routing authority — no per-node
channel arithmetic, no TX/RX channel pools, no multi-cast special cases.

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

### Why the Current Architecture Scales Poorly

In the current system, both ends of a link must agree on a topic string. The
coordinator subscribes to one wildcard per protocol per node:

```rust
for node in &nodes {
    explicit_topics.push(chardev_tx(node));        // per-node
    explicit_topics.push(reference_bus_tx(node));  // per-node (new alias)
    explicit_topics.push(can_tx(node));            // per-node
    // … one entry per protocol per node
}
```

That is O(N × P) Zenoh subscriptions (N nodes, P protocols). Adding a protocol
touches five files. Renaming a topic is a multi-file flag-day with no compiler gate.

### The Correct Mental Model

The coordinator is already the hub. Every node already connects to it with **one
socket**. The topology YAML already declares who talks to whom. The coordinator
should be the **sole routing authority** — it reads the topology at startup, assigns
stable IDs to links, and routes every frame accordingly. No node needs to know
another node's address.

### SOTA Comparison

| Framework | Connections | Routing authority |
|---|---|---|
| SystemC TLM 2.0 | Port-to-port (elaboration-time) | Compile-time socket binding |
| AUTOSAR COM | Each ECU → COM middleware | COM middleware routes by signal ID |
| SOME/IP | ECU → Service Discovery + SD | SD broker routes by service ID |
| CAN bus | Each node → bus (shared medium) | Bus arbitration by message ID |
| VirtMCU (current) | Node → Coordinator (one socket) | Coordinator guesses protocol by topic substring |
| **VirtMCU (this RFC)** | **Node → Coordinator (one socket)** | **Coordinator routes by named link_id — topology is the only authority** |

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
   Coordinator routes by (incoming link_id → topology → destination node_ids).
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
```

Link names are lowercase, hyphens allowed, no slashes. `yaml2qemu` hard-errors
on any link missing `name:` or on duplicate names within a file.

**Ordering guarantee**: link names are stable across YAML reordering. The
coordinator assigns `link_id` values from the names, not from array positions.

---

### Phase 0 — yaml2qemu: Inject `link-name` QOM Property

`yaml2qemu` injects a `link-name` QOM property into each peripheral that
participates in a topology link. This is a Stage 1 hard prerequisite.

```yaml
- name: reference_peripheral
  type: reference-peripheral
  properties:
    node-id: 0
    link-name: ref_bus        # ← injected by yaml2qemu from topology.links[name=ref_bus]
    # 'topic' is now ILLEGAL — yaml2qemu emits a hard lint error if present
```

```rust
// In the peripheral QOM struct — replaces 'topic'
#[qom_property]
pub link_name: virtmcu_qom::qom::QomString;
```

The peripheral stores `link_name` in its state. It never constructs a topic
string. `link_name` is only used in the `LinkRegistration` sent at realize time.

---

### Phase 1 — Coordinator Startup: Link ID Assignment

At startup the coordinator builds two maps:

```rust
/// link_name → link_id  (monotonically assigned from 1)
link_ids: HashMap<String, u32>

/// link_id → Vec<node_id>  (all participants; sender excluded at delivery time)
rx_map: HashMap<u32, Vec<u32>>
```

Channel 0 / link_id 0 is reserved ("no link assigned").

**Example:**

```
link_ids:  "ref_bus"     → 1
           "vehicle_can" → 2

rx_map:    1 → [0, 1]        // all participants of ref_bus
           2 → [0, 1, 2]     // all participants of vehicle_can
```

At delivery, the coordinator excludes the sender:

```rust
for &dst_node in rx_map[link_id].iter().filter(|&&n| n != src_node_id) {
    deliver_to(sockets[dst_node], link_id, src_node_id, payload);
}
```

That is the entirety of the routing logic. No protocol detection, no substring
matching, no wildcards.

**Growth**: N nodes × M links = N connections + M link IDs. For a full-mesh
topology with K nodes per link and M links, delivery is O(K) per frame — inherent
in the topology, not in the routing scheme.

---

### Phase 2 — New FlatBuffers (Wire Protocol Version Bump)

Two new tables + one enum in `hw/rust/common/virtmcu-wire/src/core.fbs`.
`UDS_PROTO_VERSION` is incremented. All schema, encoder, decoder, and coordinator
changes land in one commit (RFC-0033, §"Intra-commit lockstep").

```fbs
/// Role of this peripheral on the link.
enum LinkRole : uint8 {
  Both     = 0,   /// Sends and receives (default)
  Sender   = 1,   /// TX only — coordinator will not deliver inbound frames
  Receiver = 2,   /// RX only — peripheral has no frames to send
}

/// Node → Coordinator, topic: "sim/coord/link/register"
/// Sent once per link, after sim/coord/register, before sim/coord/start.
table LinkRegistration {
  node_id:   uint32 (required);
  link_name: string (required);   /// must match a name in topology.links
  protocol:  uint8  (required);   /// validated against topology declaration
  role:      LinkRole = Both;
}

/// Coordinator → Node, topic: "sim/coord/link/ack/{node_id}/{link_name}"
table LinkAck {
  link_id:   uint32;  /// coordinator-assigned; same value for all participants
  status:    uint8;   /// 0 = OK; non-zero = FATAL
  error_msg: string;  /// populated only when status != 0
}
```

**Coordinator behaviour on receiving `LinkRegistration`:**

1. Look up `link_name` in `link_ids`. If absent → **immediate panic**:
   `"FATAL: node {N} registered link '{name}' not in topology"`.
2. Assert `registration.protocol == topology_protocol(link_name)`. If mismatch →
   **immediate panic**:
   `"FATAL: node {N} link '{name}' declared protocol {X}, topology declares {Y}"`.
3. Assert `node_id ∈ rx_map[link_id]`. If absent → **immediate panic**:
   `"FATAL: node {N} not listed in topology participants for link '{name}'"`.
4. Mark `(node_id, link_id)` as registered.
5. Send `LinkAck { link_id, status: 0 }`.

All participants of the same link receive the same `link_id`. No per-node channel
arithmetic.

---

### Phase 3 — Pre-Flight Barrier with Timeout

After all N nodes send `sim/coord/register`, the coordinator enters the **link
registration phase**. It waits for every `(node_id, link_name)` pair in the
topology to send a matching `LinkRegistration`.

**Timeout**: 30 seconds (configurable via `--link-registration-timeout-secs`).
On expiry the coordinator **panics** with the full list of missing registrations:

```
FATAL: link registration timeout after 30s.
  Missing:
    node=0  link='ref_bus'      expected protocol=ReferenceLink
    node=2  link='vehicle_can'  expected protocol=CanFd
  Hints:
    (1) Does 'link-name' QOM property match the topology link name exactly?
    (2) Was peripheral realize() called before the timeout?
    (3) Did yaml2qemu inject 'link-name' for this peripheral?
```

Only after all registrations are received does the coordinator issue
`sim/coord/start/{node_id}` to unblock each node's VCPUs. This is the hard
pre-flight gate from RFC-0024.

---

### Phase 4 — Data Frame Format

**Node → Coordinator (TX):**

Topic: `"sim/ch/{link_id}"` — RFC-0033-framed (no framing change).

The same `link_id` is used by all senders on that link. The coordinator routing
rule:

```rust
let link_id: u32 = topic
    .strip_prefix("sim/ch/")
    .and_then(|s| s.parse().ok())
    .unwrap_or_else(|| {
        // Control-plane topics handled upstream; reaching here is a hard bug.
        panic!("FATAL: data frame on unknown topic '{topic}'");
    });

for &dst_node in rx_map[&link_id].iter().filter(|&&n| n != src_node_id) {
    deliver_to(&sockets[&dst_node], link_id, src_node_id, frame);
}
```

No `if topic.contains(…)`. No `else { Protocol::Ethernet }`. One rule, one panic,
zero silent fallbacks.

**Payload format** (unchanged from current `ZenohFrameHeader` + raw bytes):

```
┌──────────────────────────────┐
│ delivery_vtime_ns : u64 LE   │  8 bytes
│ sequence_number   : u64 LE   │  8 bytes
│ size              : u32 LE   │  4 bytes
│ payload           : [u8]     │  raw network bytes
└──────────────────────────────┘
```

**Coordinator → Node (delivery):**

Topic: `"sim/ch/{link_id}"` — same link_id, so the receiving peripheral knows
which `VtimeIngress` to dispatch to. The delivery header carries `src_node_id` for
application-level disambiguation (CAN arbitration ID, etc.).

---

### Phase 5 — VtimeIngress API Extension

```rust
impl<T: DeliveryPacket> VtimeIngress<T> {
    /// Stage 2+ API: subscribe to a coordinator-assigned link_id.
    /// The transport delivers all frames on this link to the callback,
    /// regardless of how many source nodes exist.
    pub fn new_for_link<TR, FDecode, FDeliver>(
        transport: &TR,
        link_id: u32,
        decode: FDecode,
        deliver: FDeliver,
    ) -> Self
    where
        TR: DataTransport + ?Sized,
        FDecode: Fn(&[u8]) -> Option<T> + Send + 'static,
        FDeliver: Fn(T) + Send + 'static,
    { … }

    /// Deprecated: use new_for_link().
    #[deprecated(since = "0.3.0", note = "use VtimeIngress::new_for_link()")]
    pub fn new<TR, FDecode, FDeliver>(
        transport: &TR,
        topic: &str,
        …
    ) -> Self { … }
}
```

---

### Phase 6 — DataTransport Trait Extension

```rust
pub trait DataTransport: Send + Sync {
    /// Register a link and obtain its coordinator-assigned link_id.
    fn register_link(
        &self,
        node_id:   u32,
        link_name: &str,
        protocol:  Protocol,
        role:      LinkRole,
    ) -> Result<u32, TransportError>;  // returns link_id

    /// Reserve a zero-copy buffer for a TX frame on this link.
    fn reserve_link(
        &self,
        link_id: u32,
        len: usize,
    ) -> Result<Reservation<'_>, TransportError>;

    /// Deprecated: reserve by topic string.
    /// Retained for Zenoh transport until the Zenoh follow-on RFC.
    #[deprecated(since = "0.3.0", note = "use reserve_link()")]
    fn reserve(
        &self,
        topic: &str,
        len: usize,
    ) -> Result<Reservation<'_>, TransportError>;
}
```

---

### Zenoh Path: Deprecated, Not Extended

RFC-0019 establishes UDS as the default single-host transport. The Zenoh data path
(topic-string pub/sub for cross-host) is **deprecated for simulation data frames**.

This RFC targets UDS only. The Zenoh path retains legacy topic routing because
cross-host simulations cannot pre-negotiate link IDs through a shared UDS socket.
A follow-on RFC will extend `LinkRegistration` to the Zenoh transport using a
coordinator-published channel map at federation startup.

**Enforcement**: `yaml2qemu` emits a build-time warning when the Zenoh transport
is selected: `"Zenoh data plane uses deprecated topic routing (RFC-0042 Stage 3 pending)"`.
`test_reference_ping_pong_transport_parity` continues to pass through this period.

---

### Migration Stages

| Stage | Coordinator | Peripherals | `DataTransport` | Gate |
|---|---|---|---|---|
| **1** | Dual-mode: accepts both legacy topic strings (`sim/chardev/…`) and new `sim/ch/{link_id}`; routes new topics via `rx_map` lookup + hub fan-out; adds `LinkRegistration` handler; `yaml2qemu` injects `link-name` and Zenoh deprecation warning | No change to peripheral code | `register_link()` + `reserve_link()` added; `reserve()` kept | `make test-check` green |
| **2** | New path only for migrated protocols; legacy path untouched | `reference-peripheral` first; then remaining one at a time; `VtimeIngress::new_for_link()` used; `banned_patterns` lint blocks new `topic` QOM properties | `reserve()` `#[deprecated]` | All integration tests green; zero new `topic` QOM properties in production code |
| **3** | Delete substring routing, wildcard constants, topic template functions, `base_topic` field | `topic` property removed from all peripheral structs and world YAMLs | `reserve()` removed — **only after Zenoh transport also migrated** | `make ci-full` green |

**Stage 1 is the only flag-day** (wire protocol version bump). Stages 2 and 3 are
mechanical, one peripheral at a time, independently reviewable.

---

### What Is Removed (Stage 3 Complete)

| Removed | Replaced by |
|---|---|
| `topic` QOM property on all peripherals | `link-name` (injected by `yaml2qemu`) |
| `DataTransport::reserve(topic, size)` | `DataTransport::reserve_link(link_id, size)` |
| `VtimeIngress::new(topic, …)` | `VtimeIngress::new_for_link(link_id, …)` |
| All wildcard constants (`CHARDEV_TX_WILDCARD`, …) | No wildcards — link_id is exact |
| All topic template functions (`chardev_tx`, …) | No topic templates |
| `topic.contains(…)` chain with silent Ethernet fallback | `rx_map.get(&link_id)` — panic on miss |
| `base_topic: Option<String>` in `CoordMessage` | `link_id: u32` |
| Per-node per-protocol Zenoh subscriptions | Zero Zenoh data subscriptions in UDS mode |
| Link ordering as implicit identity | `name:` field as explicit, stable identity |

---

### Updated Sequencing Diagram

```
Node 0 peripheral              Coordinator              Node 1 peripheral
       │                            │                          │
       │──sim/coord/register──►     │  ◄──sim/coord/register───│
       │                            │  (barrier: wait N nodes)
       │──sim/coord/link/register──►│  ◄──sim/coord/link/register──│
       │  {node=0, link='ref_bus',  │     {node=1, link='ref_bus',
       │   protocol=ReferenceLink,  │      protocol=ReferenceLink,
       │   role=Both}               │      role=Both}
       │                            │  validates both against topology
       │  ◄──sim/coord/link/ack/0/ref_bus──
       │    {link_id=1, status=0}   │──sim/coord/link/ack/1/ref_bus──►│
       │                            │     {link_id=1, status=0}
       │                            │   ← same link_id for all participants
       │     (pre-flight: all (node,link) pairs registered, 30s timeout)
       │                            │
       │  ◄──sim/coord/start/0──    │  ──sim/coord/start/1──────────►│
       │                            │
       │──sim/ch/1 (vtime,seq,data)►│    ← link_id=1 shared by all senders
       │──sim/coord/done/0/q/0──►   │  ◄──sim/coord/done/1/q/0───────│
       │                            │  (PDES barrier: sort by vtime,src_node,seq)
       │                            │  coordinator fans out to all other
       │                            │  participants: rx_map[1]=[0,1] → deliver to node 1
       │                            │  ──sim/ch/1 (src=0, data)──────►│
       │  ◄──sim/coord/start/0──    │  ──sim/coord/start/1──────────►│
```

## Drawbacks

**Wire protocol flag-day at Stage 1.** `UDS_PROTO_VERSION` is incremented; all
coordinator, transport, and schema changes land in one commit. Already the policy
(RFC-0033).

**`link-name` requires YAML discipline.** All topology files must add `name:` to
every link before Stage 1 lands. `yaml2qemu migrate-link-names` auto-generates
names from `{type}-{index}` for existing files as a migration aid.

**Zenoh path remains on deprecated topic routing.** Accepted temporary divergence;
the distributed scenario is rare in CI. A follow-on RFC closes the gap.

## Alternatives

**Alt A: Keep topic strings, validate at startup.**
Pre-compute all valid topics; panic on unknown. Keeps string parsing in the hot
path, keeps `base_topic`, keeps all template functions. Proliferation persists.
Rejected.

**Alt B: Per-link UDS socket.**
Each link gets its own FD. No link_id needed. Bounded by OS open-FD limit; doubles
coordinator epoll complexity; pre-flight barrier harder. Rejected.

**Alt C: Binary frame header sentinel.**
`frame_type: u8 = 0x01` as first byte conflicts with RFC-0033 `topic_len: u32 LE`
on the same socket (misparse guaranteed). Rejected in favour of `"sim/ch/{link_id}"`.

**Alt D: Per-node TX and RX channel pools.**
Earlier draft assigned separate tx_channel_id and rx_channel_id per (node, link)
pair — 2N IDs per link. `LinkAck` returned two scalars; multi-cast required a list
of rx_channel_ids. Rejected: the coordinator hub model makes per-node channels
redundant. One link_id per link suffices; the coordinator's `rx_map` handles all
fan-out.

**Cost of doing nothing:** topic aliases accumulate per release; the silent
`Protocol::Ethernet` fallback stays; every new peripheral author hits the
`chardev`/`reference_bus` class of bug.

## Prior Art

- **AUTOSAR COM**: signal routing resolved at compile time from `.arxml`; integer
  signal IDs in generated code. No runtime string lookup.
- **CAN bus**: each frame carries a message ID; the bus delivers to all listeners;
  filtering by ID is the receiver's responsibility. Direct analogue to link_id fan-out.
- **gem5**: port IDs assigned at `connectAllPorts()`; routing by integer.
- **RFC-0033 (this codebase)**: `UdsRegistration` proves typed startup-time
  negotiation is already feasible. This RFC extends the same pattern to links.

## New FlatBuffers Required

Two new tables and one enum in `hw/rust/common/virtmcu-wire/src/core.fbs`:

1. **`LinkRole`** enum — `Both | Sender | Receiver`
2. **`LinkRegistration`** table — node → coordinator at realize time
3. **`LinkAck`** table — coordinator → node, carries `link_id`

`ZenohFrameHeader` and all existing tables are unchanged. The only
backward-incompatible change is the `UDS_PROTO_VERSION` bump.

Regenerate after schema change:
```bash
cargo xtask flatc
```

## Unresolved Questions

- **`yaml2qemu migrate-link-names`**: auto-generation algorithm is
  `{link_type}-{zero_padded_index}` for deterministic output across re-runs.
  Whether names are committed as-is or require human review before merge is a
  project-policy question; resolved during Stage 1 implementation.

- **Zenoh distributed channel negotiation**: the follow-on RFC must specify how
  link IDs are agreed upon without a shared UDS socket. Leading candidate:
  coordinator publishes the full `link_id` map on a well-known Zenoh key at
  federation startup; nodes subscribe and wait before sending `LinkRegistration`.
  Out of scope here.
