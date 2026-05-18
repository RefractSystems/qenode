# RFC-0043: Coordinator Sans-I/O Architecture and Testability

**Status:** Accepted
**Created:** 2026-05-18

## 1. Objective

Refactor the central simulation coordinator into an Enterprise SOTA "Sans-I/O" state machine to eliminate asynchronous deadlocks, ensure strict protocol determinism, and enable purely synchronous, standalone unit testing of the clock and routing barriers.

## 2. Motivation

The previous iteration of the `virtmcu-coord` relied on a massive, monolithic `tokio::select!` loop combining socket I/O, Zenoh operations, logging, and core protocol state management. This led to severe technical debt:

*   **Deadlocks & Race Conditions:** Early network messages (e.g., link registrations arriving before all nodes successfully bound their UDS sockets) were dropped or mishandled.
*   **Implicit Phases:** The coordinator implicitly transitioned through phases (join -> register -> simulation) via ad-hoc `while` loops, causing dropped messages if nodes arrived out of order.
*   **Untestability:** The coordinator’s routing, handshaking, and PDES (Parallel Discrete Event Simulation) barrier logic could only be tested by spinning up full QEMU instances in integration tests, resulting in slow (~60s timeouts) and brittle debugging cycles.
*   **DI Violations:** The state machine lacked Dependency Injection, making it impossible to decouple the core logic from `tokio::net::unix::UnixStream`.

## 3. Architecture: Sans-I/O State Machine

The core logic of the coordinator MUST be separated completely from network I/O. The coordinator will be modeled as a pure `CoordinatorState` struct.

### 3.1 Event Driven Input

All incoming data from Unix sockets or Zenoh sessions is parsed at the boundary and converted into a strongly-typed `CoordinatorEvent`:

```rust
pub enum CoordinatorEvent {
    NodeJoined { node_id: u32 },
    NodeDisconnected { node_id: u32 },
    LinkRegister { node_id: u32, link_name: String },
    QuantumDone { node_id: u32, quantum: u64, vtime_ns: u64 },
    PdesMessage { 
        src_node_id: u32, 
        link_id: u32, 
        delivery_vtime_ns: u64, 
        sequence_number: u64, 
        payload: Vec<u8> 
    },
}
```

### 3.2 Action Driven Output

The state machine processes the event and returns a deterministic list of `CoordinatorAction`s to be executed by the outer I/O boundary:

```rust
pub enum CoordinatorAction {
    SendLinkAck { node_id: u32, link_id: u32 },
    BroadcastClockStart { release_quantum: u64 },
    RouteMessage { 
        target_nodes: Vec<u32>, 
        link_id: u32, 
        delivery_vtime_ns: u64, 
        sequence_number: u64, 
        payload: Vec<u8> 
    },
    AbortSimulation { reason: String },
}
```

### 3.3 Explicit Phase Model

The coordinator state machine must explicitly define its current phase. Events are only valid in certain phases; otherwise they are buffered or rejected.

```rust
pub enum CoordinatorPhase {
    AwaitingNodes { joined: HashSet<u32> },
    AwaitingLinks { registered: HashSet<(u32, String)>, remaining: HashSet<(u32, String)> },
    Simulation { current_quantum: u64 },
}
```

### 3.4 Dependency Injected Constructor

The `CoordinatorState` cannot "discover" topology or config from env vars or globals. It must be injected at construction:

```rust
pub struct LinkConfig {
    pub link_id: u32,
    pub target_nodes: Vec<u32>,
    pub delay_ns: u64,
}

pub struct CoordinatorConfig {
    pub expected_nodes: u32,
    pub links: HashMap<String, LinkConfig>,
}

impl CoordinatorState {
    pub fn new(config: CoordinatorConfig) -> Self { ... }
}
```

### 3.5 The Core Loop

The core trait implemented by the coordinator is simply:

```rust
impl CoordinatorState {
    pub fn apply(&mut self, event: CoordinatorEvent) -> Vec<CoordinatorAction> {
        // ... pure business logic based on self.phase, NO async/await, NO I/O ...
    }
}
```

## 4. Constraints and Guarantees

1.  **Zero Async in Core:** `CoordinatorState::apply` must never be `async`. It must execute in deterministic, synchronous time.
2.  **Exhaustive Standalone Tests:** The handshake (Node Joined -> Link Register -> Link Ack -> Quantum 0 Done -> Clock Start) MUST be covered by pure unit tests passing sequences of `CoordinatorEvent` and asserting on `CoordinatorAction`.
3.  **Strict PDES Execution:** The `CoordinatorState` must internally enforce the `QuantumBarrier`. It must withhold `BroadcastClockStart` until every active node has submitted `QuantumDone` for the current quantum.
4.  **Crash-Only Disconnects:** `NodeDisconnected` ALWAYS returns `AbortSimulation` during the Simulation phase. No graceful node removal is supported.
5.  **Synchronous Barrier:** The existing `QuantumBarrier` logic must be extracted as plain data inside `CoordinatorState`. `Mutex` and `Condvar` must be deleted from the barrier logic, as `apply()` is single-threaded.