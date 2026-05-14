# RFC-0020: Deterministic Test Orchestration Seeding

## Context
VirtMCU is a deterministic multi-node firmware simulation framework. One of the core mandates of the project is global simulation determinism: identical topology YAMLs, firmware ELFs, and global seeds must produce bit-identical output on every run. 

While the core runtime adheres strictly to this, the test orchestration framework (`virtmcu-test-runner`) previously relied on `rand::thread_rng()` to allocate random free TCP ports for concurrent parallel test isolation. Using wall-clock seeded random generation fundamentally breaks the ability to perfectly reproduce an orchestration sequence on different host machines or subsequent runs, even if the internal simulation itself remains deterministic.

## Decision
We enforce a strict **Global Deterministic Seeding** policy across all tooling, including the test orchestration layer:
1. The use of non-deterministic PRNGs, such as `rand::thread_rng()`, is globally banned across all crates, including the `tools/virtmcu-test-runner`.
2. Port allocation and other orchestration pseudo-randomness must derive from a deterministic base seed combined with a monotonically increasing static atomic counter.
3. By default, the base seed derives from the Process ID (`std::process::id()`). This ensures collision-free parallel execution (e.g., during `cargo test`) without blocking shared resources.
4. To allow exact reproduction of any test run (replaying the identical sequence of port bindings and test setups), the test runner must support an environment variable override: `VIRTMCU_PORT_SEED`.

## Consequences

### Positive
- **100% Reproducibility**: A failing CI pipeline can be reproduced locally bit-for-bit, port-for-port, by supplying the same `VIRTMCU_PORT_SEED`.
- **Parallel Safety**: Parallel test runs do not collide on TCP ports because each OS process receives a distinct PID default seed.
- **Strict Compliance**: The entire codebase respects the SOTA mandate of removing non-deterministic external inputs.

### Negative
- Requires passing an explicit seed during CI runs if exact post-mortem reproduction of the orchestration layer is desired.
- Port allocation logic becomes slightly more complex, utilizing hashing instead of a standard `Rng` range.
