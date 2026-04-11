# ADR-011: Zenoh as the Federation Message Bus

## Status
Accepted

## Context
In a deterministic multi-node simulation framework (`virtmcu`), multiple independent QEMU instances (cyber nodes) and physics engines (like MuJoCo) must coordinate time, exchange Ethernet frames, and transmit serial/UART bytes. 

Traditional emulation frameworks approach this in various ways:
- **Renode**: Runs everything in a single C# process space, using shared memory and direct function calls for its `WirelessMedium`.
- **SystemC / TLM-2.0**: Relies on a single C++ kernel thread to schedule and synchronize discrete events across all modules.

Because `virtmcu` relies on unmodified QEMU (which is a heavy, standalone C application with its own TCG execution loop), we cannot simply compile multiple QEMU instances into a single binary. They must run as separate processes (or separate Docker containers in a Kubernetes cluster). We need an Inter-Process Communication (IPC) layer.

## Decision
We selected **Eclipse Zenoh** (`zenoh-c` and `zenoh-rust`) as the sole federation message bus for all inter-node communication, time synchronization, and cyber-physical telemetry.

### Why not standard UDP/TCP sockets?
If Node A sends a UDP packet to Node B, the packet travels through the host operating system's network stack. The host OS scheduler introduces non-deterministic latency. Node B might receive the packet at virtual time `T=10ms` in one run, and `T=12ms` in the next run, causing the firmware to behave differently.

By routing all traffic through Zenoh, we can embed **virtual timestamps** (`delivery_vtime_ns`) into the payload headers. Node B buffers the message and only injects it into the guest firmware when QEMU's internal virtual clock catches up to the timestamp. 

### Pros
1. **High Performance and Low Overhead**: Zenoh is written in Rust and highly optimized for edge and robotics (ROS2) networks. The `zenoh-c` library exposes a zero-copy API that integrates directly into QEMU's C event loop without invoking Python.
2. **Language Agnostic**: The `TimeAuthority` can be written in Python, the `zenoh_coordinator` in Rust, and the QEMU plugins in C. They all interoperate seamlessly.
3. **Decentralized Discovery**: Zenoh automatically discovers peers on the local network via multicast. There is no central IP address or broker configuration required for basic setups, making Docker Compose orchestration trivial.
4. **Flexible Topologies**: Zenoh can route over shared memory (SHM), TCP, UDP, or QUIC. If two QEMU instances are on the same host, Zenoh uses SHM. If they are in different cloud regions, it uses TCP. The `virtmcu` code does not change.
5. **Request/Reply Semantics**: Zenoh supports synchronous `GET` queries, which perfectly fits our `zenoh-clock` requirement where QEMU must block the TCG loop and ask the `TimeAuthority` for the next time quantum.

### Cons
1. **Dependency Complexity**: Integrating a modern Rust library (`zenoh-c`) into an ancient C codebase (QEMU) requires complex CMake/Meson glue code and cross-compilation toolchains.
2. **Learning Curve**: Zenoh's concept of `KeyExpressions` (e.g., `sim/eth/frame/*/tx`) and `Queryables` is different from traditional POSIX sockets. Junior developers must learn the Zenoh API before they can contribute to the networking stack.
3. **No Native QEMU Upstream Support**: QEMU maintainers are unlikely to merge a Zenoh backend into mainline QEMU anytime soon, meaning we must maintain these patches out-of-tree via our module system (`hw/zenoh/*.so`).

## Implementation Notes for Junior Developers
If you are reading the code in `hw/zenoh/`:
- **`zenoh-clock.c`** uses Zenoh's **Queryable** API. QEMU issues a `GET` request (`z_get()`) to ask the `TimeAuthority` to advance time. It blocks until the reply is received.
- **`zenoh-netdev.c` and `zenoh-chardev.c`** use Zenoh's **Pub/Sub** API. They `z_declare_publisher()` to send outbound bytes and `z_declare_subscriber()` to receive inbound bytes asynchronously. The subscriber callback places the data in a queue, and a `QEMUTimer` is responsible for popping the queue when virtual time matches the packet's timestamp.
