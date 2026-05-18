# Docker Compose & Transport Architecture

This guide clarifies how VirtMCU's communication layers are orchestrated within Docker Compose, specifically regarding the choice between direct Zenoh connections and Unix sockets.

## The Infrastructure Flow

When deploying VirtMCU natively or within containers, the network traffic must be routed efficiently while maintaining deterministic isolation. The framework offers two primary physical transport layers: **Zenoh** and **Unix Sockets**.

### 1. Integration Testing (Local Execution)

In the Rust `native-integration` suites (`tests/native_integration/`), the transport layer is managed by the `VirtmcuTestEnv`. 
- **Zenoh** is used as the default transport even for local tests to ensure parity with the containerized environment.
- The `VirtmcuTestEnv` automatically spawns a local Zenoh router (if needed) or connects to a provided endpoint, ensuring that every test run is isolated from others.

### 2. The Docker Compose Reality (Containerized Execution)

When moving to `docker-compose.yml`, the paradigm shifts. **Docker containers operate in isolated network namespaces.**
- **Unix Sockets cannot natively span across multiple containers** without complex, brittle volume sharing (e.g., mapping `/tmp/virtmcu-sockets` across all services).
- **Zenoh** is specifically designed as a distributed, high-performance edge routing mesh over TCP/UDP. It is the State-of-the-Art (SOTA) solution for container-to-container and machine-to-machine federation.

Therefore, at the Docker Compose level, running `eclipse/zenoh:latest` directly as the `zenoh-router` service is the mandated, enterprise-grade approach.

### 3. The Node-Level Hub (`virtmcu-transport-hub`)

A common question is: *Should Docker Compose start Zenoh directly, or should it start a custom transport hub node that abstracts the transport?*

The abstraction you are looking for—preventing every single peripheral within a QEMU instance from opening its own transport connection—**already exists inside the QEMU guest**. 

It is the **`virtmcu-transport-hub` QOM device** (`hw/rust/backbone/transport-hub`).

**The Architecture:**
1. **Infrastructure Level:** Docker Compose spins up the `zenoh-router` (the physical network switch).
2. **Node Level:** The QEMU container (`cyber-node`) starts and connects to the router.
3. **Emulator Level:** Inside QEMU, you instantiate a single `-device virtmcu-transport-hub,router=tcp/zenoh-router:7447,id=hub0`. This acts as the virtual Network Interface Card (NIC) for the entire firmware instance.
4. **Peripheral Level:** All emulated peripherals (SPI, Telemetry, FlexRay) simply link to the hub using the property `transport=hub0`. They do not open their own Zenoh sessions.

### Summary

- **Do not** build a custom "Docker-level transport hub container." 
- **Do** use `eclipse/zenoh:latest` as the central federation bus.
- **Do** use `-device virtmcu-transport-hub` inside QEMU to multiplex that single Zenoh connection across all emulated peripherals for that specific node.
