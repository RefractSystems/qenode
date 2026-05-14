# RFC-0040: The Testing Pyramid and Emulation Verification

## Summary
This RFC defines the tiered testing architecture for VirtMCU, establishing clear boundaries between Unit, Integration, and End-to-End verification to ensure "Binary Fidelity" and "Global Determinism."

## Motivation
Testing a deterministic emulator is multi-dimensional. We must verify:
1. **Core Logic**: Does the Rust code correctly route packets and handle timers?
2. **Binary Fidelity**: Does a real ARM firmware binary perceive the virtual hardware as identical to silicon?
3. **Federation Integrity**: Do multiple QEMU nodes synchronize correctly under heavy load?

Without a formal pyramid, developers often over-rely on slow integration tests for simple logic bugs, or conversely, assume a unit test is enough to prove a hardware model is correct.

## The VirtMCU Testing Pyramid

### Tier 1: Unit Tests (`make test-unit`)
- **Scope**: Pure Rust logic (e.g., the `DeterministicReceiver` sorting algorithm, YAML parsing, bit-manipulation in a register).
- **Constraints**: MUST be `no_std` compatible or mock the QEMU FFI boundary. They must run in milliseconds and require no Docker/QEMU environment.
- **Tool**: `cargo test`.

### Tier 2: Miri & Sanitizers (`make test-unit-miri` / `VIRTMCU_USE_ASAN=1`)
- **Scope**: Memory safety, race conditions, and UB (Undefined Behavior).
- **Mandate**: All core synchronization primitives (`virtmcu-qom/src/sync.rs`) must pass Miri. All integration tests must run under AddressSanitizer (ASan) in CI.

### Tier 3: Native Integration Tests (`make test-integration`)
- **Scope**: The "Gold Standard" for hardware models.
- **Requirement**: Boots a real, unmodified firmware ELF (stored in `tests/firmware/`) inside a QEMU instance.
- **Verification**: Asserts on UART output, GPIO state, or network packet arrival. This proves **Binary Fidelity**.
- **Tool**: `virtmcu-test-runner` + `pytest/robot`.

### Tier 4: Multi-Node Federation Stress (`make ci-full`)
- **Scope**: Scalability and determinism of the `DeterministicCoordinator`.
- **Requirement**: Multiple QEMU nodes + a Physics engine.
- **Verification**: Ensures that the same `global_seed` produces identical results across different host CPU loads.

## Decision
1. **New Peripherals**: Every new peripheral MUST include at least one Tier 3 Integration test using a vendor SDK sample binary to prove fidelity.
2. **Deterministic Seeding**: All tests (at every tier) MUST derive their randomness from a seed that can be overridden via `VIRTMCU_GLOBAL_SEED` for post-mortem debugging.
3. **Linter Enforcement**: Lints are considered "Tier 0." They must pass before any tests are executed.

## Drawbacks
- **Binary Management**: Tier 3 tests require checking in (or managing via LFS) firmware ELFs, which increases repository size.
- **CI Complexity**: Running full QEMU-based integration tests requires containerized CI with nested virtualization support (or KVM).

## Rationale and alternatives
- **Alternative: Property-Based Testing only**: We could use `proptest` for everything. While great for logic, it cannot prove that a `SysBusDevice` is mapped at the correct address in QEMU's C memory space. We *must* boot the real emulator to verify the FFI boundary.

## Unresolved questions
- How do we handle "Flaky" tests in a deterministic framework? (Policy: In VirtMCU, there is no such thing as a "flaky" test—only a bug in the coordinator or a violation of the determinism mandates).
- Should we support automated "Bisecting" of non-deterministic failures?