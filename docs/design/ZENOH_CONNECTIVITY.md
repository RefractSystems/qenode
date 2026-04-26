# Zenoh Connectivity, Discovery, and Deterministic Pub/Sub

## 1. Context and History

VirtMCU is a deterministic co-simulation framework. To achieve determinism, all virtual time advancement and peripheral communication (Ethernet, CAN, FlexRay, LIN) are routed through Eclipse Zenoh. 

Historically, the framework suffered from "First Message Loss" (flakiness) in Continuous Integration, particularly under AddressSanitizer (ASan). Tests would occasionally timeout or fail because the very first packet emitted by QEMU (at `vtime=0`) was silently dropped.

### The "First Message" Race Condition
In a Pub/Sub topology:
1. QEMU opens a TCP socket to the Zenoh Router.
2. QEMU declares a Publisher for `sim/flexray/0/tx`.
3. QEMU puts a message on the wire.
4. If the Zenoh Router has not yet received the *Subscriber* declaration from the Python test runner (or from another QEMU node) and propagated it through its internal routing tables, **the Router drops the message** because it believes there are no downstream consumers.

Under standard execution, this race condition was rarely hit because Python was fast enough to set up the topology before QEMU finished booting. Under ASan, severe CPU throttling caused the routing table convergence to lag behind QEMU's packet emission.

---

## 2. Architectural Options Explored

To guarantee 100% delivery of the first message, we explored several synchronization strategies.

### Option A: Time-Based Polling (Legacy & Rejected)
*   **Mechanism:** `thread::sleep(100ms)` loops in QEMU, waiting for `session.info().routers_zid()` to populate.
*   **Pros:** Easy to implement.
*   **Cons:** Fails spectacularly under ASan. 100ms in wall-clock time might be 2ms in ASan execution time. It is fundamentally non-deterministic and creates "magic numbers" in the codebase.

### Option B: Reliable QoS (Rejected as a standalone fix)
*   **Mechanism:** Configure Zenoh Publishers with `Reliability::Reliable`. 
*   **Pros:** Native Zenoh feature. Ensures ordered, lossless delivery *between matched endpoints*.
*   **Cons:** It does **not** solve the discovery race. Reliable QoS only guarantees delivery to subscribers that the Publisher's router is *already aware of*. If the routing table hasn't converged, the message is still dropped at the ingress router.

### Option C: Liveliness / Topology Events (Adopted for QEMU)
*   **Mechanism:** Zenoh 1.0+ provides `session.liveliness()` and topology watchers. We can declare a callback that fires the instant a Router or Peer is discovered, signaling a `Condvar` to wake up the initialization thread.
*   **Pros:** 100% Event-driven. Zero sleeps. Instantaneous wake-up.
*   **Cons:** Slightly more complex FFI/Rust lifecycle management to avoid deadlocks.

### Option D: Wait for Matching / Strict Orchestration (Adopted for Python)
*   **Mechanism:** The test orchestrator (Python) explicitly declares all subscribers *before* launching QEMU processes, and explicitly waits for the Router to acknowledge them.
*   **Pros:** Enforces chronological correctness in the Pub/Sub mesh.

---

## 3. The VirtMCU Connectivity Protocol

To achieve **Enterprise-grade 100% correctness**, we implemented a bifurcated strategy that hides the complexity from peripheral developers while enforcing strict rules on the orchestrator.

### The Fundamental Emulator Guarantee
Peripheral developers do not need to know about Zenoh discovery. They simply call:
```rust
let session = virtmcu_zenoh::open_session(router_endpoint)?;
```
Under the hood, `open_session` executes a state-based block:
1. It opens the TCP socket.
2. It attaches a `liveliness()` subscriber to the `**` token.
3. It parks the thread on a `std::sync::Condvar`.
4. Once the Zenoh Router acknowledges the connection and sends its topology state, the callback fires, unparks the thread, and QEMU boot continues.

### The Orchestrator Guarantee
The Python test framework or external coordinator **must** establish the listening mesh before launching the devices:
1. Start Zenoh Router.
2. Python declares `session.declare_subscriber(...)`.
3. Python awaits `wait_for_zenoh_discovery(session)`, which ensures the local router has processed the declarations.
4. Python launches QEMU.

**Why this guarantees 100% correctness:** By the time QEMU's `open_session` unblocks, the Router's routing table is already fully populated with the Python orchestrator's subscribers. QEMU's first message will instantly match and route.

---

## 4. Deep Dive Scenario: Two Nodes Exchanging Data

Let's examine the lifecycle of a complete data exchange between Node 0 (TX) and Node 1 (RX) over a virtual Ethernet bus.

### Phase 1: Mesh Initialization
1.  **Coordinator Boot:** The `zenoh_coordinator` process boots, connects to the TCP router, and declares subscribers for `sim/eth/frame/*/tx`.
2.  **Node 1 (RX) Boot:** QEMU Node 1 is launched. `virtmcu-zenoh` blocks in `open_session` until the router replies with liveliness. Once unblocked, the `zenoh-netdev` peripheral initializes and calls `session.declare_subscriber("sim/eth/frame/1/rx")`.
3.  **Node 0 (TX) Boot:** QEMU Node 0 boots. It completes `open_session`, then calls `session.declare_publisher("sim/eth/frame/0/tx")`. 

*Crucially, because Node 0's publisher requires no routing-table propagation to start sending, we rely on the Coordinator being booted first to catch Node 0's packets.*

### Phase 2: Data Transmission (Node 0)
1.  **Firmware Action:** The guest firmware executing on Node 0 writes a payload to the MMIO registers of the virtual Ethernet MAC.
2.  **QOM Interception:** QEMU traps the MMIO write and passes it to the Rust `zenoh-netdev` model (while holding the Big QEMU Lock).
3.  **Serialization:** `zenoh-netdev` packages the data and the current Virtual Time into a FlatBuffer.
4.  **Zenoh Put:** The model calls `zenoh_publisher.put(flatbuffer)`. This pushes the data into Zenoh's asynchronous egress queue. *The BQL is never yielded during this step.*

### Phase 3: Routing & Coordination
1.  **Ingress:** Router 0 receives Node 0's packet. It matches the topic `sim/eth/frame/0/tx` and forwards it to the `zenoh_coordinator`.
2.  **Simulation Logic:** The coordinator applies link-layer logic (MAC address filtering, simulated cable delays, packet loss probability). 
3.  **Egress:** The coordinator rewrites the topic to `sim/eth/frame/1/rx` and publishes it back to the router.

### Phase 4: Reception (Node 1)
1.  **Zenoh Async Executor:** The Zenoh background thread inside Node 1's process receives the network packet from the socket.
2.  **Callback Execution:** The `zenoh-netdev` subscriber callback is invoked.
3.  **Thread Handoff:** *CRITICAL SAFETY BOUNDARY.* Zenoh callbacks cannot take the Big QEMU Lock (BQL), otherwise QEMU deadlocks. The callback deserializes the FlatBuffer and pushes the data into a `crossbeam_channel::unbounded` queue.
4.  **QEMU Polling/Interrupt:** A registered QEMU timer or bottom-half (BH) running on the main vCPU thread pops the data from the `crossbeam` queue. Since it is on the main thread, it already safely holds the BQL.
5.  **Hardware Injection:** The `zenoh-netdev` model writes the data into the virtual MAC's RX FIFO and calls `qemu_set_irq` to assert the hardware interrupt line.
6.  **Firmware Acknowledgment:** Node 1's guest OS jumps to the ISR, reads the MMIO FIFO, and processes the packet.

---

## 5. Known Issues & Edge Cases

While this architecture guarantees chronological correctness, the following issues can still occur:

*   **Router Death:** If the central `zenoh_router` crashes, all `open_session` connections will sever. QEMU plugins do not currently implement automatic reconnection strategies (they will panic or drop data). 
*   **Large Topologies:** In a multi-router mesh (e.g., WAN simulation), liveliness events guarantee the *local* router is reachable, but do not guarantee that the spanning tree protocol has converged across all distant routers. Tests utilizing multi-router topologies must inject an artificial `TimeAuthority` pause to allow spanning tree convergence.
*   **Multicast Storms:** Zenoh's default UDP multicast discovery is intentionally disabled in VirtMCU (`scouting/multicast/enabled=false`). If re-enabled, multiple parallel CI workers using the same multicast domain will cross-discover each other, resulting in catastrophic topic collisions and routing loops. Explicit TCP endpoints (`tcp/127.0.0.1:<port>`) are strictly mandated.