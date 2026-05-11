# Testing Strategy & Guidelines

## Quality at Scale

To maintain "Binary Fidelity" and global determinism, VirtMCU employs a multi-layered testing strategy. We prioritize automated, deterministic verification over manual inspection at every stage of the development lifecycle.


---

## 4. Test Directory Architecture

Our test suite is organized strictly by **System-Under-Test (SUT)** to maintain clear separation of concerns.

### Directory Structure

```text
tests/
├── fixtures/              # Topologies and minimal guest applications.
├── firmware/              # Golden SDK binaries for compatibility regression.
└── native_integration/    # Pure Rust (#[tokio::test]) orchestration tests.
    └── tests/             # The actual integration test files (*.rs).
```

### Pure Rust Integration Tests
Tests located in `tests/native_integration/tests/` test the interaction between **firmware**, **peripherals**, and the **coordinator**. 
* **REQUIRED**: All tests must use the `VirtmcuTestEnv` builder pattern.
* **REQUIRED**: Use `env.step_clock()` for deterministic virtual time advancement.
* **BANNED**: Manual wall-clock `thread::sleep()` or `tokio::time::sleep()`.

---

## 1. The Testing Pyramid

### Tier 0: Schema Validation (TypeSpec)
*   **TypeSpec Compilation**: Before any tests run, the `schema/world/main.tsp` is compiled. The IDL acts as a "Tier 0" test, catching structural bugs in the World Model.

### Tier 1: Unit Tests (Fast & Logic-Only)
*   **Rust**: `cargo test` within each peripheral crate. Focuses on register state machines and IRQ logic without QEMU.
*   **Lints**: Rust-based linters in `virtmcu-test-runner` verify structural integrity (FFI layouts, QOM naming, etc.).

### Tier 2: Integration Tests (QEMU + Plugins)
*   **Rust**: `cargo test -p native-integration`. Executes one or more QEMU nodes with minimal guest firmware to verify MMIO routing, clock synchronization, and peripheral registration.

### Tier 3: Multi-Node Stress Tests
*   **Rust**: Complex scenarios in `native_integration` that orchestrate multiple nodes and verify causal ordering and synchronization barrier stability under heavy host load.

---

## 2. Safe Serialization: FlatBuffers

VirtMCU uses FlatBuffers for all simulation-layer communication. Developers must **never** manipulate simulation packets using manual byte slicing.

Always use the generated Rust bindings. This ensures that any change to the `.fbs` schema is automatically caught by the compiler.

---

## 3. Deterministic Testing: The "No-Sleep" Policy

To ensure tests are 100% reproducible and immune to CI load (e.g., under ASan), **wall-clock sleeping is strictly banned.**

### 🚫 Banned: `thread::sleep` and `tokio::time::sleep`
Using `sleep` to wait for I/O or process initialization is non-deterministic. It will eventually flake.

### ✅ Mandated: Virtual Time Stepping
All time advancement must be explicitly requested via the environment:
```rust
// ✅ CORRECT: Advances the simulation clock strictly by 10ms
env.step_clock(10_000_000, 1_000_000).await?;
```

---

## 4. Timeout Scaling

VirtMCU tests are "ASan-Aware." When running under AddressSanitizer, the host CPU can be 5–10x slower. The test harness automatically scales logical timeouts. Developers should always write timeouts based on "real-time" expectations.

## 5. Local Stress Testing

When developing new features or debugging flaky tests, you must prove stability by running the test repeatedly.

```bash
# Run a specific integration test multiple times
for i in {1..20}; do cargo test -p native-integration --test uart test_uart_stress || break; done
```

## 6. The Single Simulation Entry Point (`VirtmcuTestEnv`)

The project uses a unified builder-based entry point `VirtmcuTestEnv`. This ensures a single, robust lifecycle that is immune to ordering bugs.

### The Canonical Lifecycle
The framework strictly enforces the following sequence:
1. **Spawn**: All QEMU nodes are launched frozen (`-S` is injected by the framework).
2. **Barrier**: Wait for plugin liveliness barriers across all nodes.
3. **Route**: Zenoh routing synchronization is handled internally.
4. **Init**: Initial clock synchronization executes while nodes are still frozen.
5. **Start**: QMP `cont` is issued to all nodes simultaneously.
6. **Teardown**: Strict RAII-based teardown on exit.

### Usage Patterns

#### Single-Node Simulation
```rust
#[tokio::test]
async fn test_peripheral() -> Result<()> {
    VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_firmware_path("path/to/firmware.elf")
                .with_dtb_path("path/to/board.dtb")
        )
        .run_test(|env| Box::pin(async move {
            env.step_clock(1_000_000, 100_000).await?;
            Ok(())
        })).await;
    Ok(())
}
```

#### Multi-Node Simulation
```rust
#[tokio::test]
async fn test_network() -> Result<()> {
    VirtmcuTestEnv::builder()
        .add_node(NodeConfig::new(0).with_firmware_path("f0.elf").with_dtb_path("d0.dtb"))
        .add_node(NodeConfig::new(1).with_firmware_path("f1.elf").with_dtb_path("d1.dtb"))
        .run_test(|env| Box::pin(async move {
            // All nodes spawn frozen, complete the barrier sequence,
            // then `cont` is issued simultaneously to all of them.
            env.step_clock(1_000_000, 100_000).await?;
            Ok(())
        })).await;
    Ok(())
}
```

### Banned Patterns in Tests
- **Manual `-S` in `qemu_args`**: Framework-injected; manual override breaks synchronization logic.
- **Direct `Command::new("qemu-system-arm")`**: Use `VirtmcuTestEnv` for any test that executes guest code.
- **Manual QMP `cont`**: Emulation start must be coordinated by the `VirtmcuTestEnv` lifecycle.

---

## 7. Automated Flight Recorder (PCAP)

Whenever a `native-integration` test fails, the harness can be configured to dump the network traffic history into a PCAP file.

### Locating Artifacts
Artifacts are typically saved to the `target/test-results/` directory when enabled.

---

## 8. When to write a Lint, a Test, or a Postmortem

| Artifact | When to use it | Goal |
| :--- | :--- | :--- |
| **Lint** | When a bug is caused by a **static disagreement** between files (e.g., name mismatch, layout drift). | Fail at **lint time** (`virtmcu-test-runner lint`). |
| **Unit Test** | When a bug is in **internal logic** (e.g., a state machine transition). | Fail during `cargo test`. |
| **Integration Test** | When a bug is in the **interaction** between components (e.g., QEMU ↔ Zenoh). | Fail during `cargo test -p native-integration`. |
| **Postmortem** | When a bug is **complex, cascading, or structural**. | Documentation for **future engineers**. |

### The "Fail Loudly" Principle
If a bug can be caught at lint time, **write a linter**. Do not rely on a runtime test to catch a name mismatch that will only surface as a SIGSEGV in a different part of the system.

---

## 10. Rust Test Runner (`virtmcu-test-runner`)

The `virtmcu-test-runner` is the primary CLI tool for running lints and managing the simulation environment.

### Usage
```bash
# Run all lints
cargo run -p virtmcu-test-runner -- lint

# Run a specific YAML test specification
cargo run -p virtmcu-test-runner -- run --spec tests/specs/my_test.yaml
```

