# Observability & AI Co-pilot

## Seeing the Unseen

As VirtMCU evolves from a foundational emulator into a robust digital twin environment, observability and AI accessibility become first-class concerns. We provide deep introspection without embedding complex GUIs into the emulator core.

---

## 1. High-Fidelity Telemetry

VirtMCU provides rich, interactive observability into the guest's execution. By tracing CPU sleep states, peripheral events, and register mutations, we publish deterministic timelines over the simulation bus.

### Event Streaming
The `hw/rust/observability/telemetry` layer publishes low-overhead FlatBuffer events:
- **CPU State**: Tracking when a node enters/exits `WFI` (Wait For Interrupt).
- **IRQ Tracing**: Recording exactly when and why an interrupt line was asserted.
- **Peripheral Events**: High-level semantic logs (e.g., "UART FIFO Full", "CAN ID Matched").

These streams can be ingested by visual timeline tools or analyzed by the test harness to verify complex timing requirements.

---

## 2. Distributed Observability (OpenTelemetry)

### Design Goal

Every virtmcu process — QEMU nodes, time authority, deterministic coordinator, test runner — emits structured traces and logs into a single, time-ordered view. The view is keyed on two timestamps:

| Timestamp | Source | Meaning |
|---|---|---|
| `wall_time` | OTel SDK | Real-world nanoseconds; used for Collector routing and Grafana display. |
| `vtime_ns` | simulation clock | Virtual nanoseconds; deterministic simulation order. |

Both are carried as span attributes so queries can order by either axis.

### Architecture

```
┌─────────────────────────────────────────────────────────────┐
│ Per-process (binary or QEMU plugin)                         │
│                                                             │
│   tracing::{error!, warn!, info!, debug!}                   │
│            ↓                                                │
│   tracing-opentelemetry layer (RAII guard, init at start)   │
│            ↓                                                │
│   opentelemetry-otlp (gRPC, port 4317)                      │
└─────────────────────┬───────────────────────────────────────┘
                      │ OTLP/gRPC
                      ▼
            ┌─────────────────────┐
            │   OTel Collector    │  docker service: otel-collector
            │   receives OTLP     │
            │   enriches attrs    │
            │   routes:           │
            │     logs  → Loki    │
            │     traces → Tempo  │
            └──────┬──────┬───────┘
                   │      │
            ┌──────┘      └──────┐
            ▼                    ▼
        ┌────────┐          ┌─────────┐
        │  Loki  │          │  Tempo  │
        │ (logs) │          │ (traces)│
        └────────┘          └─────────┘
              └──────┬───────────┘
                     ▼
                 ┌─────────┐
                 │ Grafana │  unified view, port 3000
                 └─────────┘
```

### Shared init — `virtmcu-observability` crate

All binaries call a single entry point at startup:

```rust
// tools/virtmcu-observability/src/lib.rs
pub fn init_telemetry(service_name: &'static str) -> impl Drop {
    // Wires tracing-subscriber → tracing-opentelemetry → opentelemetry-otlp
    // Endpoint: OTEL_EXPORTER_OTLP_ENDPOINT env var (default: http://otel-collector:4317)
    // Returns a guard; dropping it flushes pending spans before process exit.
}
```

Each binary:
```rust
fn main() {
    let _telemetry = virtmcu_observability::init_telemetry("virtmcu-physical-node");
    // ...
}
```

### Service names

| Process | Service name |
|---|---|
| `virtmcu-physical-node` | `virtmcu-physical-node` |
| `deterministic_coordinator` | `virtmcu-deterministic-coordinator` |
| `virtmcu-test-runner` | `virtmcu-test-runner` |
| `virtmcu-cli` | `virtmcu-cli` |
| TCG tracer plugin | `virtmcu-qemu-plugin-{node_id}` |

### vtime_ns propagation

Services that advance or observe the simulation clock record the current virtual time on every span:

```rust
tracing::info!(vtime_ns = resp.current_vtime_ns(), "quantum complete");
```

This allows Grafana/Loki queries to sort logs by simulation time rather than wall time, which is essential for deterministic replay analysis.

### TCG Plugin — two-tier logging

The TCG tracer plugin (`hw/rust/observability/tcg-tracer`) requires different logging behaviour depending on when in its lifecycle the event occurs:

| Phase | Mechanism | Reason |
|---|---|---|
| Inside `qemu_plugin_install` (pre-init) | `qemu_plugin_outs()` | Synchronous, always visible. QEMU logs to its own output channel before any Rust subscriber exists. |
| After successful `STATE.set()` | `tracing::error!` via OTel subscriber | `SimpleSpanProcessor` (not batch) ensures flush before abnormal exit. |

```rust
// qemu_plugin.rs — QEMU plugin API binding
extern "C" {
    pub fn qemu_plugin_outs(string: *const c_char);  // synchronous, pre-subscriber safe
}

// lib.rs — used only inside qemu_plugin_install
macro_rules! plugin_log {
    ($($arg:tt)*) => {{
        let msg = alloc::format!("{}\0", alloc::format!($($arg)*));
        unsafe { qemu_plugin_outs(msg.as_ptr().cast()) };
    }};
}
```

### QEMU pause-mode boundary

QEMU always starts frozen (`-S`). Plugin initialization and `qemu_plugin_install` run **before** QEMU reaches the paused state. The OTel pipeline is therefore not yet connected for pre-pause errors. The `qemu_plugin_outs` path covers this gap; OTel handles everything after the `cont` QMP command is issued.

```
QEMU starts
  → device realize (QOM)
  → qemu_plugin_install()   ← plugin_log! here (pre-pause)
  → enters paused state     ← OTel subscriber initialized here
  → QMP "cont" from orchestrator
  → firmware runs           ← tracing::* here (OTel captures)
```

### Graceful degradation — OTel is always optional

The OTLP exporter is fire-and-forget. If the Collector is unreachable the exporter
silently drops spans; it never blocks or panics the binary. A `fmt` stdout layer is
always active as a fallback, so `tracing::info!` / `error!` output appears on stdout
in every configuration.

| Configuration | Behaviour |
|---|---|
| Collector running | stdout (fmt layer) **+** Loki/Tempo via Collector |
| Collector absent | stdout (fmt layer) only |
| `OTEL_SDK_DISABLED=true` | stdout (fmt layer) only, OTel SDK fully bypassed |

The `OTEL_SDK_DISABLED=true` environment variable (standard OTel spec) disables the
SDK without any code change. Set it when running unit tests or in CI environments that
have no Collector.

### docker-compose overlay — observability is opt-in

The Collector/Loki/Tempo/Grafana services live in a **separate overlay file** so the
base standalone compose never requires them:

```bash
# Standalone — no telemetry stack needed:
docker-compose -f docker/docker-compose.yml up

# With full observability dashboard:
docker-compose -f docker/docker-compose.yml -f docker/docker-compose.observability.yml up
```

`docker/docker-compose.observability.yml`:

```yaml
services:
  otel-collector:
    image: otel/opentelemetry-collector-contrib:0.103.0
    volumes:
      - ./otel-collector-config.yaml:/etc/otelcol-contrib/config.yaml:ro
    ports:
      - "4317:4317"   # OTLP gRPC
    networks:
      - virtmcu-net
    depends_on:
      - loki
      - tempo

  loki:
    image: grafana/loki:3.0.0
    command: ["-config.file=/etc/loki/local-config.yaml"]
    networks:
      - virtmcu-net

  tempo:
    image: grafana/tempo:2.5.0
    command: ["-config.file=/etc/tempo.yaml"]
    networks:
      - virtmcu-net

  grafana:
    image: grafana/grafana:11.0.0
    ports:
      - "3000:3000"
    environment:
      GF_AUTH_ANONYMOUS_ENABLED: "true"
      GF_AUTH_ANONYMOUS_ORG_ROLE: Admin
    volumes:
      - ./grafana-datasources.yaml:/etc/grafana/provisioning/datasources/datasources.yaml:ro
    networks:
      - virtmcu-net
    depends_on:
      - loki
      - tempo

networks:
  virtmcu-net: {}
```

The base `docker/docker-compose.yml` is **not modified** by OTel work.

### Workspace dependencies to add

```toml
# Cargo.toml [workspace.dependencies]
opentelemetry            = { version = "0.27", features = ["trace"] }
opentelemetry-otlp       = { version = "0.27", features = ["trace", "grpc-tonic"] }
opentelemetry_sdk        = { version = "0.27", features = ["trace", "rt-tokio"] }
tracing-opentelemetry    = "0.28"
```

### Implementation status

| Component | Status |
|---|---|
| `qemu_plugin_outs` binding + `plugin_log!` macro | ✅ Done |
| `tracing`/`tracing-subscriber` in workspace deps | ✅ Done |
| `virtmcu-observability` shared init crate | ✅ Done |
| OTel workspace deps | ✅ Done |
| Binary init calls (time-authority, coordinator, test-runner, cli) | ✅ Done |
| TCG plugin post-init OTel subscriber | ✅ Done |
| docker-compose (Collector, Loki, Tempo, Grafana) | ✅ Done |
| Grafana dashboard / datasource config | ✅ Done |

---

## 3. AI Co-pilot & MCP

To support LLM-driven debugging and lifecycle management, VirtMCU is designed to interface with the **Model Context Protocol (MCP)**. This allows AI agents to act as "peer programmers" in the simulation environment by providing semantic access to the simulation state.

### Capabilities for AI Agents:
- **Control**: AI agents can provision boards, flash firmware, and control node lifecycle (start/stop/pause) via `virtmcu-run`.
- **Introspection**: Agents can inspect raw memory, read CPU registers, and disassemble guest code dynamically via the `virtmcu-cli qmp` tool or the `QmpClient` integrated into `virtmcu-test-runner`.
- **Interactive Debugging**: Agents can interact with UART consoles, monitor network traffic, and inject faults to verify firmware resilience.

---

## 4. Semantic Debugging

Because VirtMCU is deterministic, we can perform **Record & Replay** debugging.
1. **Record**: Run a simulation and log all telemetry and network traffic to a PCAP or JSON oracle.
2. **Analyze**: An AI agent or human engineer analyzes the trace to identify the exact virtual nanosecond where a bug occurred.
3. **Replay**: Re-run the simulation with the same seed and a GDB debugger attached. The bug will manifest at the exact same point, every time.

This removes the "Heisenbug" problem from embedded software development, making even the most complex multi-node races reliably reproducible.

### PCAP Link-Layer Schema and Wireshark Integration
VirtMCU exports binary network and telemetry traces using the **DLT_USER0 (147)** link layer. We multiplex the different node protocols via a 2-byte protocol identifier immediately following the standard 8-byte src/dst routing header:
- Protocol `1`: Ethernet
- Protocol `2`: UART
- Protocol `3`: IEEE 802.15.4
- Protocol `4`: CAN-FD
- Protocol `5`: FlexRay
- Protocol `255`: VirtMCU Test Infrastructure (topics, direction markers).

**Live Observability**:
To bring this data to life, VirtMCU provides deep integration with Wireshark:
- **Extcap Plugin**: The `tools/wireshark/virtmcu_extcap.py` plugin enables Wireshark to natively capture live traffic streaming over the VirtMCU Zenoh bus.
- **Lua Dissector**: The `tools/wireshark/virtmcu_dissector.lua` script provides high-level protocol decoding within the Wireshark UI, perfectly aligning the capture timestamps with the simulation's deterministic `delivery_vtime_ns`.
- **Offline Analysis**: Alternatively, traffic can be recorded headlessly using the `virtmcu-cli debug pcap-dump` utility for later analysis.
