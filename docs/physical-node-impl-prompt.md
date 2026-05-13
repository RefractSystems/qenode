# Implementation Prompt: Physical Node Refactor

## Overview

This prompt refactors `virtmcu-time-authority` into `virtmcu-physical-node` — a CPS-native
binary where **the physical plant owns virtual time**. In Cyber-Physical Systems, time
progresses because the physical world steps forward; the cyber node (QEMU firmware) is a
reactor, not the clock source. This refactor makes that relationship explicit in the code.

This document is self-contained. Read it fully before writing a single line.

---

## Preconditions

The following must be true before starting:

1. `docs/physics-gateway-impl-prompt.md` is fully applied and `make test-check` passes.
2. `hw/rust/common/virtmcu-api/src/physics_generated.rs` exists (generated from `physics.fbs`).
3. Traits `PhysicsGatewayTransport` and `PhysicsGatewayServer` exist in `virtmcu-api`.
4. `tools/cyber_bridge/src/bin/physics_gateway.rs` exists with the SHM + futex logic.
5. `ctrlc` is in `tools/cyber_bridge/Cargo.toml` (add if missing: `ctrlc = "3"`).

Verify: `cargo build -p cyber_bridge 2>&1 | head -5` must print no errors.

---

## Mandatory Pre-Flight

Before writing any code, answer these three questions:

1. **Architectural Alignment**: Does every `PhysicalNode` implementation receive its
   transports and session exclusively via constructor arguments — no global state, no
   lazy singletons?
2. **Fail Loudly**: If `--plant embedded` is passed but `--n-sensors` is zero when
   sensors are expected, does the binary `panic!`/`expect!` rather than silently
   produce garbage data?
3. **Verification Gate**: `make test-check` after each phase.

---

## Risks and DRY/RAII Invariants

The following pitfalls must be avoided:

| Risk | Mitigation |
|---|---|
| `ActuatorBuffer` type alias in `cyber_bridge/src/lib.rs` diverges from `ActuatorMap` in `virtmcu-api` | Delete local alias; re-export `virtmcu_api::ActuatorMap` |
| `EmbeddedPlant` still holds `Arc<zenoh::Session>` after refactor | Drop the session field; sensor publishing moves to the binary's main loop |
| `RemotePlant` returns no sensors but the binary tries to publish them | `PlantState.sensors` is empty for `RemotePlant`; gateway publishes sensors directly |
| `worlds/pendulum_controller.yml` `mock-physics` has `depends_on: time-authority` | Update that dependency to `physical-node` in the same commit |
| New `pub` items in `virtmcu-api` trigger `missing_docs` denial | Every new `pub` struct, type alias, and trait item must carry a doc comment |
| `virtmcu-api` is `no_std`-compatible; `HashMap` needs `std` for `RandomState` | Inner actuator map uses `BTreeMap<u32, Vec<f64>>` — alloc-only |
| `PhysicsStep` trait deleted while `time_authority.rs` still imports it | Rename the file last, after all implementations migrate to `PhysicalNode` |

---

## Phase 1 — `virtmcu-api`: `ActuatorMap`, `SensorMap`, `PlantState`, `PhysicalNode`

**File**: `hw/rust/common/virtmcu-api/src/lib.rs`

Add the following after the existing transport traits. All items are `pub` and all must
have doc comments (the crate denies `missing_docs`).

```rust
/// Causal actuator commands for one quantum.
///
/// Outer key: delivery virtual time (ns) at which firmware issued the command.
/// Inner key: actuator index as declared in the board topology YAML.
/// Inner value: actuator data words (length = `data_size` from topology).
///
/// Use `BTreeMap` (not `HashMap`) throughout — this type must be `no_std`-compatible.
pub type ActuatorMap = alloc::collections::BTreeMap<
    u64,
    alloc::collections::BTreeMap<u32, alloc::vec::Vec<f64>>,
>;

/// Sensor readings produced by one quantum step of the physical plant.
///
/// Key: sensor index as declared in the board topology YAML.
/// Value: sensor data words (length = `data_size` from topology).
///
/// Empty for `RemotePlant` — the Physics Gateway publishes sensors directly.
pub type SensorMap = alloc::collections::BTreeMap<u32, alloc::vec::Vec<f64>>;

/// State produced by one quantum step of the physical plant.
pub struct PlantState {
    /// Virtual time (ns) at the END of the completed quantum.
    pub vtime_ns: u64,
    /// Sensor readings to publish on `sim/sensor/{node}/sensordata_{i}`.
    ///
    /// Empty when the plant delegates to an external Physics Gateway process,
    /// which publishes sensors directly. Non-empty for in-process (`EmbeddedPlant`).
    pub sensors: SensorMap,
}

/// The physical world in a Cyber-Physical System simulation.
///
/// Owns virtual time progression and plant dynamics for exactly one simulation node.
/// The binary's main loop calls `step()` once per quantum, after:
/// 1. Issuing `ClockAdvanceReq` and receiving `ClockReadyResp` from the QEMU cyber node.
/// 2. Draining the `ZenohActuatorSink` to obtain the complete actuator bundle.
///
/// Implementations: `TickOnlyPlant`, `EmbeddedPlant`, `RemotePlant`.
pub trait PhysicalNode: Send + Sync {
    /// Advance the plant by one quantum.
    ///
    /// `quantum_ns` is the size of the completed quantum in nanoseconds.
    /// `actuators` contains all firmware commands delivered during this quantum,
    /// ordered by `(delivery_vtime_ns, actuator_id)`.
    ///
    /// Returns the updated `PlantState` or a fatal error string. Callers treat
    /// any `Err` as a simulation abort — do not retry.
    fn step(
        &mut self,
        quantum_ns: u64,
        actuators: &ActuatorMap,
    ) -> Result<PlantState, alloc::string::String>;
}
```

Run `make test-check` — must pass before proceeding.

---

## Phase 2 — `cyber_bridge/src/lib.rs`: Canonical `ActuatorMap`

**File**: `tools/cyber_bridge/src/lib.rs`

Replace the local type alias with a re-export so there is one canonical definition:

```rust
// DELETE this line:
// type ActuatorBuffer = std::collections::BTreeMap<u64, std::collections::HashMap<u32, Vec<f64>>>;

// ADD this re-export:
pub use virtmcu_api::ActuatorMap;
```

Update `ZenohActuatorSink`:
- Change the `buffer` field type from `Arc<Mutex<ActuatorBuffer>>` to
  `Arc<Mutex<ActuatorMap>>`.
- Update the subscriber callback's inner map insertion: replace any `HashMap::entry()`
  with `BTreeMap::entry()` (same API surface; `BTreeMap` is a drop-in here).
- Update `drain()` return type from `ActuatorBuffer` to `ActuatorMap`.

The unit test `test_zenoh_actuator_sink` must still pass unchanged.

Run `make test-check`.

---

## Phase 3 — `cyber_bridge/src/physics.rs`: Migrate to `PhysicalNode`

This phase renames the three existing types and makes them implement `PhysicalNode`
instead of `PhysicsStep`. Delete `PhysicsStep` once all callers are migrated.

### 3A — `TickOnlyPlant` (was `NoOpPhysics`)

```rust
/// A physical plant that advances virtual time but models no dynamics.
///
/// Use for pure cyber-node testing and nodes with no actuators or sensors.
pub struct TickOnlyPlant;

impl virtmcu_api::PhysicalNode for TickOnlyPlant {
    fn step(
        &mut self,
        _quantum_ns: u64,
        _actuators: &virtmcu_api::ActuatorMap,
    ) -> Result<virtmcu_api::PlantState, alloc::string::String> {
        Ok(virtmcu_api::PlantState {
            vtime_ns: 0, // caller tracks absolute vtime; this field unused for TickOnly
            sensors: virtmcu_api::SensorMap::new(),
        })
    }
}
```

### 3B — `EmbeddedPlant` (was `SharedMemPhysics`)

**Remove** the `session: Arc<zenoh::Session>` and `topic_prefix: String` fields from the
struct. Sensor publishing moves to the binary's main loop.

Updated struct:

```rust
pub struct EmbeddedPlant {
    mmap: MmapMut,
    shm_path: std::path::PathBuf,
    node_id: u32,
    n_sensors: u32,
    n_actuators: u32,
    timeout_ms: u64,
    bridge_seq: u32,
}
```

Updated `new()` signature (drop `session` and `topic_prefix` parameters):

```rust
pub fn new(
    node_id: u32,
    n_sensors: u32,
    n_actuators: u32,
    timeout_ms: u64,
) -> anyhow::Result<Self>
```

Updated `PhysicalNode` implementation — replace the `PhysicsStep` impl.
The step logic is identical to the existing `SharedMemPhysics::step()` through the futex
wait (steps 1–3 in that implementation). **Replace step 4** (Zenoh publish) with building
the return value:

```rust
// 4. Read sensor values from SHM and return them in PlantState
let mut sensors = virtmcu_api::SensorMap::new();
for i in 0..self.n_sensors {
    let offset = SHM_DATA_OFFSET + (i as usize) * 8;
    let val = f64::from_le_bytes(
        self.mmap[offset..offset + 8].try_into().expect("SHM sensor slice is 8 bytes"),
    );
    sensors.insert(i, alloc::vec![val]);
}
Ok(virtmcu_api::PlantState {
    vtime_ns: resp_vtime_ns, // pass the quantum-end vtime from the caller
    sensors,
})
```

Note: the `step()` signature no longer receives `&ClockReadyResp`. Instead it receives
`quantum_ns: u64` and `actuators: &ActuatorMap` per the trait. The quantum-end vtime
must be tracked by the caller (the binary main loop). Adjust the SHM range selection:

```rust
// quantum_start and quantum_end computed by the caller and passed as part of actuators
// (the ActuatorMap is already range-scoped by the binary's main loop)
// EmbeddedPlant takes the last value per actuator_id from the full map:
let mut ctrl_values: std::collections::BTreeMap<u32, f64> = std::collections::BTreeMap::new();
for (_vtime, id_map) in actuators {
    for (&id, vals) in id_map {
        if let Some(&v) = vals.first() {
            ctrl_values.insert(id, v);
        }
    }
}
```

Keep the `Drop` impl unchanged.

### 3C — `RemotePlant` (was `GatewayPhysics`)

Rename and update the `PhysicsStep` impl to `PhysicalNode`:

```rust
/// A physical plant that delegates dynamics to an external Physics Gateway process.
///
/// Sends a `PhysicsTrigger` FlatBuffer to the gateway and blocks until `PhysicsDone`
/// is received. The gateway publishes sensor data directly to Zenoh; this struct
/// returns an empty `sensors` map.
pub struct RemotePlant {
    transport: Box<dyn virtmcu_api::PhysicsGatewayTransport>,
    timeout: std::time::Duration,
    quantum_number: u64,  // tracked here so the trigger matches ClockAdvanceReq
}

impl RemotePlant {
    /// Creates a new `RemotePlant`.
    pub fn new(
        transport: Box<dyn virtmcu_api::PhysicsGatewayTransport>,
        timeout_ms: u64,
    ) -> Self {
        Self {
            transport,
            timeout: std::time::Duration::from_millis(timeout_ms),
            quantum_number: 0,
        }
    }
}
```

`PhysicalNode::step()` for `RemotePlant`:
1. Build `PhysicsTrigger` from `actuators` exactly as the current `GatewayPhysics::step()`.
2. Call `self.transport.trigger_and_wait(trigger_bytes, self.timeout)`.
3. Increment `self.quantum_number`.
4. Return `PlantState { vtime_ns: 0, sensors: SensorMap::new() }`.

### 3D — Delete `PhysicsStep`

Remove the `PhysicsStep` trait definition and its three `impl` blocks. Confirm no other
files import it: `grep -r "PhysicsStep" --include="*.rs" .` must return no results.

Run `make test-check`.

---

## Phase 4 — `physics_gateway.rs`: Add Sensor Publishing

**File**: `tools/cyber_bridge/src/bin/physics_gateway.rs`

The gateway currently reads sensor values from SHM but does not publish them to Zenoh.
This means sensors never reach firmware when using `RemotePlant`. Fix this.

### 4A — Add Zenoh args

```rust
/// Zenoh endpoint for publishing sensor data. Optional; omit to skip sensor publishing.
#[arg(long)]
data_connect: Option<String>,

/// Topic prefix for sensor publications (default: sim/sensor).
#[arg(long, default_value = "sim/sensor")]
sensor_prefix: String,
```

### 4B — Open Zenoh session if `--data-connect` is provided

After parsing args, open an optional Zenoh session:

```rust
let zenoh_session: Option<Arc<zenoh::Session>> = if let Some(ref endpoint) = args.data_connect {
    let mut config = virtmcu_zenoh_config::client_config();
    let json_connect = format!("[\"{endpoint}\"]");
    config
        .insert_json5("connect/endpoints", &json_connect)
        .map_err(|e| anyhow::anyhow!("Zenoh config error: {e}"))?;
    Some(Arc::new(zenoh::open(config).wait().map_err(|e| anyhow::anyhow!("{e}"))?))
} else {
    None
};
```

### 4C — Publish sensors after each SHM step

In the main loop, after `shm.step(trigger, args.timeout_ms)` succeeds and before
`server.send_done(done)`, add:

```rust
if let Some(ref session) = zenoh_session {
    for i in 0..args.n_sensors {
        let offset = cyber_bridge::physics::SHM_DATA_OFFSET + (i as usize) * 8;
        // safety: shm.mmap is valid for the duration of this block
        let val_bytes = shm.sensor_bytes(i); // add a helper method (see below)
        let topic = format!("{}/{}/sensordata_{}", args.sensor_prefix, args.node_id, i);
        let payload = virtmcu_api::encode_frame(trigger.quantum_end_vtime_ns(), 0, val_bytes);
        session
            .put(&topic, payload)
            .wait()
            .map_err(|e| anyhow::anyhow!("Zenoh sensor publish failed: {e}"))?;
    }
}
```

Add `GatewayShm::sensor_bytes(i: u32) -> &[u8]` helper:

```rust
pub fn sensor_bytes(&self, sensor_index: u32) -> &[u8] {
    let offset = SHM_DATA_OFFSET + (sensor_index as usize) * 8;
    &self.mmap[offset..offset + 8]
}
```

Make `SHM_DATA_OFFSET` accessible from this binary by importing it from
`cyber_bridge::physics`.

Run `make test-check`.

---

## Phase 5 — Rename binary and update main loop

### 5A — `Cargo.toml`

**File**: `tools/cyber_bridge/Cargo.toml`

Replace:

```toml
[[bin]]
name = "virtmcu-time-authority"
path = "src/bin/time_authority.rs"
```

With:

```toml
[[bin]]
name = "virtmcu-physical-node"
path = "src/bin/physical_node.rs"
```

### 5B — Rename the source file

```bash
git mv tools/cyber_bridge/src/bin/time_authority.rs \
        tools/cyber_bridge/src/bin/physical_node.rs
```

### 5C — Update CLI args in `physical_node.rs`

Replace the `PhysicsType` enum and `--physics` arg:

```rust
#[derive(Debug, Clone, ValueEnum)]
enum PlantType {
    /// Advance virtual time only; no physics dynamics.
    TickOnly,
    /// In-process SHM plant: write actuators to /dev/shm, read sensors back.
    Embedded,
    /// Delegate to an external Physics Gateway process via transport.
    Remote,
}
```

```rust
/// Physical plant implementation
#[arg(long, value_enum, default_value_t = PlantType::TickOnly)]
plant: PlantType,
```

Remove the old `--physics` arg and its `PhysicsType` enum entirely.

### 5D — Update plant construction

Replace the `match args.physics { ... }` block with:

```rust
let mut plant: Box<dyn virtmcu_api::PhysicalNode> = match args.plant {
    PlantType::TickOnly => Box::new(cyber_bridge::physics::TickOnlyPlant),
    PlantType::Embedded => {
        Box::new(cyber_bridge::physics::EmbeddedPlant::new(
            args.node_id,
            args.n_sensors,
            args.n_actuators,
            args.timeout_ms,
        )?)
    }
    PlantType::Remote => {
        let transport: Box<dyn virtmcu_api::PhysicsGatewayTransport> =
            match args.gateway_transport {
                TransportType::Unix => {
                    let path = args.gateway_connect.as_ref().ok_or_else(|| {
                        anyhow::anyhow!("--gateway-connect required for Remote plant with Unix transport")
                    })?;
                    Box::new(cyber_bridge::physics_transport::UnixSocketPhysicsTransport::new(path))
                }
                TransportType::Zenoh => {
                    let session = if let Some(ref s) = zenoh_session {
                        Arc::clone(s)
                    } else {
                        open_zenoh_session(args.gateway_connect.as_ref()).await?
                    };
                    Box::new(cyber_bridge::physics_transport::ZenohPhysicsTransport::new(session))
                }
            };
        Box::new(cyber_bridge::physics::RemotePlant::new(transport, args.timeout_ms))
    }
};
```

### 5E — Update the main quantum loop

The main loop currently calls `transport.advance()` then `physics.step()` separately.
After this change, the structure is:

```rust
let mut quantum_number: u64 = 0;
let mut absolute_vtime_ns: u64 = 0;

loop {
    // 1. Issue ClockAdvanceReq to the QEMU cyber node
    let req = ClockAdvanceReq::new(args.delta_ns, absolute_vtime_ns, quantum_number);
    let resp = /* ... transport.advance(req, timeout) with retry-at-zero logic ... */;

    if resp.error_code() == CLOCK_ERROR_STALL {
        anyhow::bail!("Clock stall at quantum {quantum_number}");
    }

    // 2. Drain actuators for this quantum window
    let actuators = actuator_sink
        .as_ref()
        .map(|s| s.drain())
        .unwrap_or_default();

    // 3. Step the physical plant
    let plant_state = plant
        .step(args.delta_ns, &actuators)
        .map_err(|e| anyhow::anyhow!("Plant step failed at quantum {quantum_number}: {e}"))?;

    // 4. Publish sensors returned by in-process plant (EmbeddedPlant)
    if let Some(ref session) = zenoh_session {
        for (&sensor_id, vals) in &plant_state.sensors {
            let topic = format!("{}/{}/sensordata_{}", args.sensor_prefix, args.node_id, sensor_id);
            let mut bytes: Vec<u8> = Vec::with_capacity(vals.len() * 8);
            for &v in vals {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
            let vtime_end = absolute_vtime_ns + args.delta_ns;
            let payload = virtmcu_api::encode_frame(vtime_end, 0, &bytes);
            session
                .put(&topic, payload)
                .wait()
                .map_err(|e| anyhow::anyhow!("Sensor publish failed: {e}"))?;
        }
    }

    quantum_number += 1;
    absolute_vtime_ns += args.delta_ns;
}
```

Add `--sensor-prefix` arg (mirrors the `physics_gateway` arg, used for `EmbeddedPlant`):

```rust
/// Topic prefix for sensor publications when using the embedded plant.
#[arg(long, default_value = "sim/sensor")]
sensor_prefix: String,
```

### 5F — Update startup log line

```rust
info!(
    plant = ?args.plant,
    node_id = args.node_id,
    delta_ns = args.delta_ns,
    "Starting Physical Node"
);
```

Run `make test-check`.

---

## Phase 6 — Update `worlds/*.yml`

### `worlds/pendulum.yml`

1. Rename service `time-authority` → `physical-node`.
2. Update binary path from `virtmcu-time-authority` to `virtmcu-physical-node`.
3. Add `--plant tick-only` (was implicit noop — now explicit).

```yaml
  physical-node:
    # ... same build/network/depends_on/volumes as before ...
    command: [
      "/app/target/release/virtmcu-physical-node",
      "--transport", "zenoh",
      "--connect", "tcp/zenoh-router:7447",
      "--node-id", "0",
      "--delta-ns", "1000000",
      "--plant", "tick-only"
    ]
```

### `worlds/pendulum_controller.yml`

1. Rename service `time-authority` → `physical-node`.
2. Update binary path.
3. Replace `--physics shm` with `--plant embedded`.
4. **Critical**: update `mock-physics.depends_on` from `time-authority` to `physical-node`.

```yaml
  physical-node:
    # ... same build/network/depends_on/volumes as before ...
    command: [
      "/app/target/release/virtmcu-physical-node",
      "--transport", "zenoh",
      "--connect", "tcp/zenoh-router:7447",
      "--node-id", "0",
      "--delta-ns", "1000000",
      "--timeout-ms", "30000",
      "--plant", "embedded",
      "--n-sensors", "1",
      "--n-actuators", "1"
    ]

  mock-physics:
    # ...
    depends_on:
      physical-node:           # was: time-authority
        condition: service_started
```

---

## Phase 7 — Update Documentation

Apply the following targeted changes. Do not rewrite sections wholesale — make surgical
edits to keep diff size minimal and reviewable.

### `docs/architecture/07-cyber-physical-integration.md`

In §4 "Simulation Modes / Integrated Mode":
- Replace `virtmcu-time-authority` with `virtmcu-physical-node`.
- Replace "Time Authority" with "Physical Node" in the same paragraph.

### `docs/architecture/12-physics-gateway.md`

1. **§1.2 Mermaid "After" diagram**: rename `virtmcu-time-authority` node to
   `virtmcu-physical-node`.
2. **§2 Component Roles mermaid**: replace `TA["virtmcu-time-authority…"]` label with
   `PN["virtmcu-physical-node…"]` and update all `TA` node references to `PN`.
3. **§9 topology diagrams** (§9 "Deployment Topologies"): replace "Time Authority" box
   labels with "Physical Node".
4. **§10 Zenoh Topic Map**: no change needed (topics are stable).
5. **See Also** link to `physics-gateway-impl-prompt.md`: no change.

### `docs/architecture/02-temporal-core.md`

- Replace `virtmcu-time-authority` binary references with `virtmcu-physical-node`.
- The concept name "TimeAuthority" (e.g. "The Golden Rule … Physical Node acting as the
  TimeAuthority") is historical; update to "Physical Node" throughout.

### `docs/architecture/13-standards-alignment.md`

Update the FMI row:

```
| **Co-simulation master** | Physical Node (`virtmcu-physical-node`) |
```

### `docs/tutorials/lesson08-zenoh-clock/README.md`

Replace all occurrences of `virtmcu-time-authority` with `virtmcu-physical-node` and
`TimeAuthority` with `Physical Node`.

### `docs/tutorials/lesson12-cyber-physical-bridge/README.md`

Same substitutions. Update the command example:

```bash
target/release/virtmcu-physical-node \
  --transport zenoh \
  ...
```

### `docs/adr/011-zenoh-federation-bus.md`

Replace the `TimeAuthority` mentions with `Physical Node`.

---

## Verification Gates

After each phase:
```bash
make test-check   # lint + unit; must exit 0
```

After Phase 5 (binary rename), additionally verify the binary builds:
```bash
cargo build -p cyber_bridge --bin virtmcu-physical-node
```

Confirm the old binary no longer exists:
```bash
cargo build -p cyber_bridge --bin virtmcu-time-authority 2>&1 | grep "no bin target"
```

After Phase 6 (worlds YAML), verify Docker Compose parses cleanly:
```bash
docker compose -f worlds/pendulum_controller.yml config --quiet
```

Full CI parity before PR:
```bash
make ci-check
```

---

## Sequencing Constraint

This prompt must be applied **before** `docs/federation-transfer-impl-prompt.md`.
The federation-transfer prompt targets `physical_node.rs` and `virtmcu-physical-node`
by name. Applying federation-transfer first, then this prompt, would require re-editing
the same lines twice.

After this prompt, the binary name `virtmcu-time-authority` must not appear anywhere in
the repository (except git history):

```bash
grep -r "virtmcu-time-authority" --include="*.rs" --include="*.toml" \
     --include="*.md" --include="*.yml" --include="*.yaml" . | grep -v ".git"
# Must produce no output.
```
