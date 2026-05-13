# Implementation Prompt: The Federation Transfer

## Context

This prompt is self-contained. Read it fully before writing a single line.

VirtMCU uses a three-tier vocabulary for the simulation container concept:

- **World** (`world.yaml`) — the static YAML manifest: nodes, topology, seed.
  Analogous to an HLA Federation Object Model. **Never rename schema fields.**
- **Federation** — the *running instance* of a World. Term adopted from IEEE Std 1516
  (HLA). Identified at runtime by `--federation-id`. Lives only in CLI flags, log
  output, and diagnostic APIs. Does not appear in Rust struct names or YAML keys.
- **Stage** — reserved for the future OpenUSD `UsdStage` path (ADR-010 roadmap).

The "federation transfer" work introduces `--federation-id` as a first-class CLI concept
across the Physical Node, Physics Gateway, and Deterministic Coordinator, and ensures
every component emits it in logs and Zenoh session metadata. It does NOT rename any
existing Rust types, YAML keys, or Zenoh topic patterns.

**Precondition**: The Physical Node refactor (`docs/physical-node-impl-prompt.md`) must
be complete and `make test-check` must pass before applying this prompt. The binary is
named `virtmcu-physical-node` and lives in
`tools/cyber_bridge/src/bin/physical_node.rs`.

---

## Pre-Flight Checklist (CLAUDE.md mandated)

1. **Architectural Alignment**: All changes use DI (federation-id injected at
   construction, not discovered). No global static variables.
2. **Fail Loudly**: If `--federation-id` is required but absent, clap exits with a
   non-zero status and prints the flag name automatically. No silent defaults that mask
   misconfiguration.
3. **Verification Gate**: `make test-check` (lint + unit) must pass with zero new
   warnings. Targeted integration: `virtmcu-test-runner --test federation_id`.

---

## Phase 1 — Shared Library: `federation-id` type in `virtmcu-api`

**File**: `hw/rust/common/virtmcu-api/src/lib.rs`

Add a newtype so the federation ID is not a bare `String` everywhere:

```rust
/// Opaque identifier for a running simulation instance (IEEE HLA "federation").
/// Injected at startup via --federation-id; never discovered at runtime.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FederationId(pub String);

impl FederationId {
    /// Returns the federation ID as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(feature = "std")]
impl std::fmt::Display for FederationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
```

Note: `Display` is gated on `feature = "std"` because `virtmcu-api` is `no_std`-compatible.
This type belongs in `virtmcu-api` because every binary (Physical Node, Gateway,
Coordinator) will receive it as a constructor argument.

---

## Phase 2 — Physical Node CLI

**File**: `tools/cyber_bridge/src/bin/physical_node.rs`

The binary is procedural (no outer struct). Add `--federation-id` directly to the `Args`
struct and emit it in the main loop.

### 2.1 Add `--federation-id` to `clap` args

Locate the `Args` struct. Add:

```rust
/// Identifier for this running simulation instance (HLA: federation name).
/// Used in log output and Zenoh session metadata. Required.
#[arg(long, env = "VIRTMCU_FEDERATION_ID")]
federation_id: String,
```

### 2.2 Construct the `FederationId` newtype at startup

In `main()`, immediately after parsing args:

```rust
let federation_id = virtmcu_api::FederationId(args.federation_id.clone());
```

### 2.3 Emit federation ID in the quantum loop

At the top of the main quantum `loop { … }` block that issues `ClockAdvanceReq`, add:

```rust
tracing::info!(
    federation = %federation_id,
    quantum = quantum_number,
    "quantum start"
);
```

Use `tracing` structured fields — do not interpolate into the message string.

### 2.4 Zenoh session metadata (best-effort)

When opening the Zenoh session, set user-info metadata so operators can identify
sessions in `zenoh ls`:

```rust
let _ = config.insert_json5(
    "metadata/federation_id",
    &format!("\"{}\"", args.federation_id),
);
```

A config insertion failure must not abort startup.

---

## Phase 3 — Physics Gateway CLI

**File**: `tools/cyber_bridge/src/bin/physics_gateway.rs`

Add the same `--federation-id` arg pattern as Phase 2. The gateway uses it:

1. In log lines: every `tracing::info!` / `tracing::warn!` emits
   `federation = %federation_id`.
2. The SHM path remains `/dev/shm/virtmcu_physics_{node_id}` — the federation ID
   does **not** go into the SHM path. It appears only in diagnostic output.
3. In the `PhysicsDone` acknowledgment log.

---

## Phase 4 — Deterministic Coordinator CLI

**File**: wherever the coordinator's `main()` lives (search for `DeterministicCoordinator`
in `tools/`).

Same pattern: add `--federation-id`, construct `FederationId` newtype, emit in structured
logs.

Additionally, the Coordinator should validate at startup that every node declared in the
World YAML has registered within `--join-timeout-ms` milliseconds. The error message must
include the federation ID:

```rust
anyhow::bail!(
    "Federation {}: node '{}' did not join within {}ms",
    federation_id,
    missing_node_id,
    join_timeout_ms,
);
```

---

## Phase 5 — QEMU Plugin Tracing (Cyber Nodes)

**File**: `hw/rust/common/virtmcu-qom/src/clock.rs` (or wherever
`ClockSyncResponder` is instantiated inside QEMU).

The QEMU plugin receives a `--federation-id` parameter via the `-device` property string:

```
-device virtmcu-clock,mode=slaved-suspend,federation_id=run-42
```

Add a `federation_id` property to the QOM device:

```rust
// In the OBJECT_PROPERTY_LIST or equivalent Rust QOM property registration:
DEFINE_PROP_STRING("federation_id", VirtmcuClock, federation_id),
```

In `QemuBridge::log_quantum()` (or equivalent), emit the federation ID as a structured
field alongside the quantum number.

**Note**: The federation ID must **not** be part of the Zenoh topic structure. Topic
patterns like `sim/clock/advance/{node_id}` are stable; the federation ID is metadata
only.

---

## Phase 6 — Documentation Cross-References

### 6.1 `docs/architecture/01-system-overview.md`

Already updated with the World / Federation / Stage section (see current file state).
No additional changes needed.

### 6.2 `docs/architecture/12-physics-gateway.md`

In §2 (Component Roles), update the Mermaid graph node label for `virtmcu-physical-node`
to show it accepts `--federation-id`:

```
PN["virtmcu-physical-node<br/>--federation-id &lt;id&gt;<br/>ZenohActuatorSink collects commands<br/>Issues ClockAdvanceReq / ClockReadyResp<br/>Sends PhysicsTrigger after quantum"]
```

### 6.3 `docs/architecture/13-standards-alignment.md`

Update the FMI co-simulation master row:

```
| **Co-simulation master** | Physical Node (`virtmcu-physical-node`) |
```

### 6.4 `docs/SUMMARY.md`

Already updated to include `13-standards-alignment.md`. No additional changes needed.

---

## Phase 7 — Integration Test

**File**: `tests/integration/federation_id_test.rs` (or equivalent in the test runner).

```rust
#[tokio::test]
async fn test_federation_id_propagates_to_logs() {
    let fed_id = "test-federation-001";
    let mut pn = spawn_physical_node(&["--federation-id", fed_id, /* … */]).await;
    let log_line = pn.next_log_line_matching("quantum start").await;
    assert!(log_line.contains(fed_id),
        "Expected federation ID '{}' in log: {}", fed_id, log_line);
    pn.kill().await;
}

#[test]
fn test_federation_id_required() {
    let output = std::process::Command::new("virtmcu-physical-node")
        .arg("--world").arg("test_world.yaml")  // deliberately omit --federation-id
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("federation-id") || stderr.contains("VIRTMCU_FEDERATION_ID"),
        "Error message must mention the missing flag: {}", stderr);
}
```

---

## Invariants — What Must NOT Change

| Invariant | Enforcement |
|---|---|
| `WorldSpec` Rust struct name unchanged | `grep -r "WorldSpec" --include="*.rs"` must still find existing usages |
| `world.yaml` key names unchanged | YAML round-trip test must pass |
| Zenoh topic patterns unchanged | `sim/clock/advance/{node}` etc. are stable wire protocol |
| SHM file name unchanged | `/dev/shm/virtmcu_physics_{node_id}` — no federation ID in path |
| No new `#[allow(…)]` suppressions | `make test-lint` must pass |

---

## Verification Gate

```bash
# Fast path
make test-check

# Full CI parity
make ci-check

# Targeted
virtmcu-test-runner --test federation_id
```

All three must exit 0 before the PR is opened.
