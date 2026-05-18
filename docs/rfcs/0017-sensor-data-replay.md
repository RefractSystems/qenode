# RFC-0017: Enterprise Sensor Data Replay & Trace Formats

## Status
Accepted

## Context
VirtMCU currently utilizes the Renode Sensor Data Format (RESD) for simple time-series sensor injections. While sufficient for basic educational use cases and lightweight simulation, RESD fails to meet Enterprise Grade SOTA (State-of-the-Art) requirements for scalability, schema-evolution, high-frequency indexing, and integration with modern automotive and robotics ecosystems.

Simultaneously, VirtMCU is aligning with OpenUSD (Universal Scene Description) for defining the spatial-temporal "World State" (see RFC-0010, RFC-0016). However, storing high-frequency telemetry (e.g., 1MB/s UART dumps, CAN-FD traces, or I2C sensor bursts) inside an OpenUSD scene graph causes severe binary bloat and performance degradation during rendering and querying.

Research into Enterprise SOTA practices—specifically those utilized by Siemens (Simcenter/PreScan) and MathWorks (MATLAB/Simulink)—reveals a standardized bifurcation of data formats:
1. **ASAM MDF4 (.mf4):** The "Gold Standard" in the automotive industry (OEMs and Tier-1s) for high-speed bus logging (CAN, LIN, FlexRay) and raw signal tracing.
2. **MCAP (.mcap):** The modern SOTA for robotics and autonomous systems, featuring high-performance O(1) seeking, embedded schemas, and native integration with visualization tools like Foxglove and ROS2.
3. **ASAM OSI (Open Simulation Interface):** A Protobuf-based standard used for defining "Ground Truth" object lists and sensor detections (commonly wrapped in MCAP or HDF5).

To enable VirtMCU to act as a Deterministic Co-Simulation node within these enterprise ecosystems (e.g., replaying a real-world car trace into virtual firmware), we must formally define our data ingestion and replay architecture.

## Decision
We will deprecate RESD as the primary trace format and adopt a **Hybrid Replay Architecture** that aligns with industry standards while maintaining VirtMCU's strict determinism requirements.

### 1. Primary Container: MCAP
*   **Role:** The native container format for all VirtMCU telemetry recordings and high-bandwidth sensor replays.
*   **Why:** MCAP is format-agnostic, allowing us to embed our existing Flatbuffers schemas directly into the file. It is heavily supported in the Rust ecosystem (`mcap` crate) and integrates seamlessly with our OpenTelemetry observability stack.

### 2. Automotive Standard Support: ASAM MDF4 and ASAM OSI
*   **Role:** The ingested format for Hardware-in-the-Loop (HIL) and vehicle network traces.
*   **Why:** Siemens Testlab, Vector CANoe, and MATLAB natively export to MDF4. We will support this ecosystem via a conversion or adapter layer (`mdf2mcap`), ensuring VirtMCU can consume real-world drive data. ASAM OSI will be used when injecting pre-processed object detections (Sensor-in-the-Loop).

### 3. Separation of Concerns (OpenUSD vs. Traces)
*   **OpenUSD (`.usd`)** will be strictly reserved for **Macro-Architecture** (World State, 3D Geometry, Kinematics).
*   **MCAP (`.mcap`)** will be strictly reserved for **Micro-Architecture Traces** (High-frequency bus traffic, register states, raw sensor arrays).
*   The OpenUSD scene will contain metadata tags pointing to the associated MCAP files, keeping the scene graph lightweight.

### 4. Deterministic Replay Injection
*   We will implement a `virtmcu-replay` node (a dedicated Zenoh client).
*   This node will participate in the Chandy-Misra-Bryant (CMB) quantum barrier managed by the `DeterministicCoordinator`.
*   It will read chunks of the MCAP file and publish the sensor data to the Zenoh bus with the exact `delivery_vtime_ns` required for bit-identical simulation reproducibility.

## Consequences
*   **Positive:** VirtMCU will seamlessly integrate with Foxglove Studio for tracing, and MATLAB/Simulink for co-simulation workflows.
*   **Positive:** We avoid the massive binary bloat and performance overhead of forcing OpenUSD to act as a time-series database.
*   **Negative:** Adds implementation complexity; we must build or integrate an MCAP parser in Rust and ensure it aligns perfectly with the virtual time (`vtime_ns`) abstraction.


