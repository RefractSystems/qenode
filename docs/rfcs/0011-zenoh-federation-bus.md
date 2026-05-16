# RFC-0011: Zenoh as the Federation Message Bus

## Status
Accepted, **partially superseded by RFC-0019** for single-host deployments.
Zenoh remains canonical only for cross-host federation. The UDS Hybrid IPC
(RFC-0019) plus zero-copy `DataTransport` (RFC-0025) is the default for the
single-host path. The `ZENOH_TOPIC_MAP.md` referenced below was never
checked in; treat it as a TODO for whoever revisits cross-host federation.

## Context
In a deterministic multi-node simulation framework (`VirtMCU`), multiple independent QEMU instances (cyber nodes) and physics engines (like MuJoCo) must coordinate time, exchange Ethernet frames, and transmit serial/UART bytes. 

Traditional emulation frameworks approach this in various ways:
- **Renode**: Runs everything in a single C# process space, using shared memory and direct function calls for its `WirelessMedium`.
- **SystemC / TLM-2.0**: Relies on a single C++ kernel thread to schedule and synchronize discrete events across all modules.

Because `VirtMCU` relies on unmodified QEMU (which is a heavy, standalone C application with its own TCG execution loop), we cannot simply compile multiple QEMU instances into a single binary. They must run as separate processes (or separate Docker containers in a Kubernetes cluster). We need an Inter-Process Communication (IPC) layer.

## Decision
We originally selected **Eclipse Zenoh** (native Rust) as the sole federation message bus for all inter-node communication, time synchronization, and cyber-physical telemetry.

*Update*: The architecture has since evolved into a **Dual-Transport Architecture**. Zenoh remains the primary Data Plane transport for distributed, multi-node federations and canonical message sorting. However, for 1:1 Control Plane communication (e.g., Clock synchronization and MMIO socket bridging) on a single host, we now also support and prefer **Unix Domain Sockets** to achieve single-digit microsecond latency.

### Why not standard UDP/TCP sockets?
If Node A sends a UDP packet to Node B, the packet travels through the host operating system's network stack. The host OS scheduler introduces non-deterministic latency. Node B might receive the packet at virtual time `T=10ms` in one run, and `T=12ms` in the next run, causing the firmware to behave differently.

By routing all traffic through Zenoh, we can embed **virtual timestamps** (`delivery_vtime_ns`) into the payload headers. Node B buffers the message and only injects it into the guest firmware when QEMU's internal virtual clock catches up to the timestamp. 

### Pros
1. **High Performance and Low Overhead**: Zenoh is written in Rust and highly optimized for edge and robotics (ROS2) networks. Native Rust plugins integrate directly into QEMU's event loop.
2. **Language Agnostic**: The `Physical Node` can be written in Python, the `deterministic_coordinator` in Rust, and the QEMU plugins in Rust. They all interoperate seamlessly.
3. **Flexible Discovery**: Zenoh supports both decentralized discovery (multicast) and explicit endpoints. VirtMCU strictly mandates explicit TCP/UDP endpoints for deterministic CI execution.
4. **Flexible Topologies**: Zenoh can route over shared memory (SHM), TCP, UDP, or QUIC. If two QEMU instances are on the same host, Zenoh uses SHM. If they are in different cloud regions, it uses TCP. The `VirtMCU` code does not change.
5. **Request/Reply Semantics**: Zenoh supports synchronous `GET` queries, which perfectly fits our `clock` requirement where QEMU must block the TCG loop and ask the `Physical Node` for the next time quantum.

### Cons
1. **Toolchain Complexity**: Integrating a modern Rust library into QEMU requires managing Rust cross-compilation toolchains alongside the standard C toolchain.
2. **Learning Curve**: Zenoh's concept of `KeyExpressions` (e.g., `sim/eth/frame/*/tx`) and `Queryables` is different from traditional POSIX sockets.
3. **No Native QEMU Upstream Support**: QEMU maintainers are unlikely to merge a Zenoh backend into mainline QEMU anytime soon, meaning we must maintain these patches out-of-tree via our module system (`hw/rust/*.so`).

## Implementation Notes for Junior Developers
If you are reading the code in `hw/rust/`:
- **`clock`** uses Zenoh's **Queryable** API. QEMU issues a `GET` request to ask the `Physical Node` to advance time. It blocks until the reply is received.
- **`netdev` and `chardev`** use Zenoh's **Pub/Sub** API, but **Direct Pub/Sub between simulation nodes is STRICTLY BANNED** for data-plane traffic. To preserve PDES tie-breaking and per-quantum barrier synchronization, all outbound bytes are published to the `DeterministicCoordinator`. The Coordinator buffers, sorts, and forwards the packets. The peripheral's subscriber callback (via `DeterministicReceiver`) places the inbound data in a queue, and a `QEMUTimer` is responsible for popping the queue when virtual time matches the packet's timestamp.

## External References
* For a complete mapping of all active Zenoh topics in the system, see [Zenoh Topic Map](ZENOH_TOPIC_MAP.md).
