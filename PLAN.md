# Migration Plan: Rust White-Box Tests

**Objective:** Migrate white-box invariant tests (protocol contracts, topic registries, and PDES concurrency logic) from Python orchestration to native Rust `cargo test`. This adheres to the Bifurcated Testing Strategy, reducing QEMU boot overhead and utilizing tools like `loom` and `miri` for deeper correctness validation.

## Target Architecture

| Python Origin | Target Rust Location | Testing Tool | Purpose |
| :--- | :--- | :--- | :--- |
| `test_topic_registry.py` | `virtmcu_qom` (Topics module) | `cargo test` | Pin string literals to Rust enums/macros. Ensure cross-language topic agreement. |
| `test_vproto.py` | `virtmcu_qom` (Protocol module) | `cargo test` | Verify binary packing/unpacking offsets, endianness, and buffer boundaries. |
| `test_coordinator_barrier.py` | `tools/deterministic_coordinator/src/barrier.rs` | `cargo test` + `loom` | Internal unit tests and permutation testing for race conditions in the barrier logic. |
| `test_coordinator_barrier.py` (Int.) | `tools/deterministic_coordinator/tests/test_ordering.rs` | `cargo test` | Pure-Rust integration test using a fake/in-memory transport to verify TX/DONE ordering. |

## Execution Phases

### Phase 1: Shared Constants & Contracts (Current)
**Goal:** Guarantee that Python and Rust never diverge on topic strings or binary protocol structures.
1.  **Protocol Set/De:** Locate the Rust structs (`MmioReq`, `ClockAdvanceReq`, `ZenohFrameHeader`) and add a `#[cfg(test)]` module. Implement tests asserting memory layout, endianness, and known byte-array conversions.
2.  **Topics Registry:** Create/update Rust-side representations of `SimTopic`. Add tests ensuring output matches the exact string literals pinned in Python.

### Phase 2: PDES Coordinator Concurrency
**Goal:** Reproduce TX/DONE ordering bugs deterministically without QEMU.
1.  **Loom Stress Tests:** Wrap the `QuantumBarrier`'s internal state in `loom` primitives and test concurrent signaling.
2.  **Pure-Rust Integration Tests:** Create `test_canonical_ordering.rs` to spin up the coordinator with an in-memory transport, injecting out-of-order messages to assert canonical delivery.

### Phase 3: FFI Boundary Hardening
**Goal:** Ensure memory safety across the C/Rust QOM boundary.
1.  **Layout Verification:** Ensure all QOM state structs use `static_assertions::assert_eq_size!` and `assert_eq_align!`.
2.  **Miri Validation:** Ensure FFI initialization and teardown routines run cleanly under `cargo miri test`.

### Phase 4: Deprecation and CI Cleanup
**Goal:** Remove redundant Python tests and verify CI efficiency.
1.  **Deprecation:** Delete migrated tests (e.g., `test_vproto.py`) from the Python suite.
2.  **Validation:** Run `make ci-full` to confirm improved test suite execution times.
