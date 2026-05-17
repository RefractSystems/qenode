# RFC-0040: The Testing Pyramid and Emulation Verification

## Summary

This RFC defines the tiered testing architecture for VirtMCU, establishes the criteria for each tier, and mandates which tier must be satisfied before a feature or peripheral ships. The goal is to ensure "Binary Fidelity" and "Global Determinism" are empirically proven, not assumed.

## Motivation

Testing a deterministic emulator is multi-dimensional. Three distinct properties must each be verified independently:

1. **Core Logic**: does the Rust code correctly route packets, handle timers, and sort delivery queues?
2. **Binary Fidelity**: does a real ARM firmware binary perceive the virtual hardware as identical to silicon?
3. **Federation Integrity**: do multiple QEMU nodes synchronize correctly under load, and does the same `global_seed` produce bit-identical output?

Without a formal pyramid, developers over-rely on slow integration tests for simple logic bugs, or conversely, assume a unit test is sufficient to prove a hardware model is correct. Both are wrong and expensive.

## Decision: The VirtMCU Testing Pyramid

### Tier 0: Linters

Linters are not tests — they are build gates. `make test-lint` (`clippy --deny warnings`, `cargo fmt --check`) must pass before any other tier runs. Suppressing a lint to make a build green is equivalent to deleting a test.

### Tier 1: Unit Tests (`make test-unit`)

**What they verify**: pure Rust logic in isolation — sorting algorithms in `VtimeIngress`, YAML parsing, register bitfield manipulation, serialization round-trips.

**Why this tier exists**: these run in milliseconds, require no QEMU binary, and catch regressions in the 90% of code that has nothing to do with the FFI boundary. Catching a bug here is orders of magnitude cheaper than catching it in Tier 3.

**Constraints**: must be `no_std` compatible or mock the QEMU FFI boundary. No Docker, no QEMU process.

### Tier 2: Miri and Sanitizers (`make test-unit-miri` / `VIRTMCU_USE_ASAN=1`)

**What they verify**: memory safety, data races, and undefined behavior that Rust's type system cannot statically rule out (particularly around `unsafe` in `virtmcu-qom/src/sync.rs`).

**Why this tier exists**: the BQL is a C-level lock; Rust cannot prove correctness across the FFI boundary by type checking alone. Miri and AddressSanitizer provide the empirical safety evidence.

### Tier 3: Native Integration Tests (`make test-integration`)

**What they verify**: Binary Fidelity — an unmodified, vendor-supplied firmware ELF perceives the virtual peripheral as equivalent to the real silicon. Asserts on UART output, GPIO state, or transport packet timing.

**Why this tier exists**: no amount of unit testing can prove that `SysBusDevice` is mapped at the correct address in QEMU's C memory space, or that register reset values match the datasheet. Only booting the real emulator with real firmware proves this.

**Gate**: every new peripheral must include at least one Tier 3 test using a vendor SDK sample binary.

### Tier 4: Multi-Node Federation Stress (`make ci-full`)

**What they verify**: scalability and determinism of `DeterministicCoordinator` under load. Same `global_seed` → bit-identical output across multiple host CPU loads.

**Why this tier exists**: determinism bugs in the PDES barrier (quantum pre-increment, tie-breaking violations) only manifest under concurrent multi-node load. They cannot be exercised by any lower tier.

## Drawbacks

- **Binary Management**: Tier 3 tests require checking in (or managing via LFS) firmware ELFs, which increases repository size.
- **CI Complexity**: Tier 3 requires containerized CI with QEMU support; Tier 4 requires multi-process orchestration.

## Rationale and Alternatives

**Alternative: Property-Based Testing (`proptest`) only**: powerful for logic, but cannot prove that a `SysBusDevice` is mapped at the correct address in QEMU's C memory space. The real emulator boot is not optional for Binary Fidelity. Rejected as a replacement; accepted as a complement to Tier 1.

**Alternative: Only integration tests**: too slow for the inner development loop. The pyramid structure ensures fast feedback at each layer. Rejected.

## Unresolved Questions

None. The question "how do we handle flaky tests?" has a determined answer: in VirtMCU, a "flaky" test reveals a determinism bug in the coordinator or a violation of the PDES mandates. The correct response is to fix the non-determinism, not to mark the test as flaky or add retries.

## Related

- RFC-0006: Binary Fidelity (the mandate Tier 3 enforces)
- RFC-0001: Core Constraints (determinism mandate that Tier 4 enforces)
- RFC-0020: Deterministic Test Orchestration Seeding (global seed override)
