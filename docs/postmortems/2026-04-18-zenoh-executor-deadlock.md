# Postmortem: Zenoh Executor Deadlock & Topology Chaos

**Date:** April 18, 2026  
**Author:** Gemini CLI  
**Component:** `virtmcu-zenoh` and `zenoh-clock` QEMU plugins (Rust)  

## The Incident
During the integration of virtmcu into the wider FirmwareStudio digital twin platform, simulation environments (like the Inverted Pendulum) completely hung during the boot sequence. Time inside QEMU failed to advance, and the `physics-node` master clock repeatedly reported `ReplyError { payload: "Timeout" }` when sending clock advance `GET` requests to QEMU.

## The Investigation
The symptoms were confusing:
1. QEMU successfully connected to the Zenoh router.
2. The `physics-node` observed that QEMU was "READY".
3. But when `physics-node` sent `sim/clock/advance/0`, it timed out.

I suspected an issue with the Zenoh queryable. We injected `zenoh-python` listener scripts to verify connectivity, but queries consistently failed to receive a response from QEMU. 

Tracing into the QEMU plugin source code (`hw/rust/zenoh-clock/src/lib.rs`), I found this callback:
```rust
let queryable = session
    .declare_queryable(topic)
    .callback(move |query| {
        let backend = unsafe { &*(backend_ptr as *const ZenohClockBackend) };
        on_clock_query(backend, query);
    })
    .wait()
    .unwrap();
```
Inside `on_clock_query`, the code did this:
```rust
query.reply(query.key_expr(), resp_bytes.as_slice()).wait().unwrap();
```

**The Root Cause (Deadlock):** Zenoh 1.x query handlers are executed by the Zenoh runtime's internal thread pool. By calling `.wait().unwrap()` on the `.reply()` future *inside* the callback itself, we were blocking an executor thread. Under load or specific router conditions, this caused an executor deadlock, freezing the plugin and silently dropping all clock advance replies.

**The Secondary Cause (Topology Chaos):** In `virtmcu-zenoh/src/lib.rs`, QEMU was connecting using Zenoh's default `peer` mode with multicast scouting and shared memory enabled. Because FirmwareStudio runs `mmio_bridge`, `cyber_agent`, and QEMU inside the *same* Docker container, they were all discovering each other over loopback/shared memory and forming a chaotic peer-to-peer mesh, rather than cleanly routing traffic through the designated `zenoh-router:7447` infrastructure. 

## The Fixes
1. **Non-blocking Callbacks:** Removed `.wait().unwrap()` from the `zenoh-clock` reply handler. We wrapped the reply in a lightweight `std::thread::spawn` (or an async task) so it could execute without blocking the Zenoh executor.
2. **Graceful Init:** Replaced `declare_queryable(...).wait().unwrap()` with a proper `match` block. If the router rejects the declaration, QEMU logs an error and exits gracefully rather than panicking.
3. **Topology Isolation:** Updated `virtmcu-zenoh` `open_session()` to explicitly force `"mode": "client"`, and disable `scouting/multicast` and `transport/shared_memory` when an explicit router is provided. This forces a clean star topology.

## Developer Tips for Debugging Distributed Systems
* **Never block an async executor:** If you are inside a callback provided by a networking library (like Zenoh, gRPC, or Tokio), never call synchronous `.wait()`, `.recv()`, or `thread::sleep()`. You will inevitably cause a deadlock. Offload the work to a background thread or use proper asynchronous `.await` chaining.
* **Isolate your local networks:** When running multiple IPC-heavy processes in a single container or host, defaults like "auto-discovery" and "shared memory" can create unintended feedback loops. Explicitly configure your nodes as `clients` connecting to a single broker/router unless you explicitly need a decentralized mesh.
* **Expect the network to fail:** A `unwrap()` during initialization (like opening a session or declaring a listener) assumes the infrastructure is perfect. It never is. Handle connection timeouts and rejections gracefully so the system can tear down cleanly, providing actionable logs instead of a silent panic.