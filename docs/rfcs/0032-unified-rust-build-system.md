# RFC-0032: Unified Rust Build System (xtask)

## Status
Accepted

## Context
Historically, the VirtMCU build pipeline relied on a fragile combination of shell scripts (`build.sh`, `gen_trigger.sh`) and Makefiles bridging Meson and Cargo. This led to cross-platform compatibility issues, inconsistent compiler flag application (like ASan/TSan), and non-deterministic build environments.

## Decision
We adopt the Rust `xtask` pattern combined with a unified `virtmcu-cli` tool. All complex build orchestration, schema compilation (TypeSpec to JSON/Rust), and code generation (C module triggers) are migrated from Bash/Python to pure Rust.

## Reference-level explanation
- **`xtask`**: A Cargo alias that executes a local Rust binary to orchestrate the build. It ensures that environments, flags, and dependencies are managed securely and predictably.
- **`virtmcu-cli`**: Exposes commands like `setup patch-qemu` and `gen --typespec`. It is responsible for generating the `modinfo.json` trigger files so QEMU's dynamic loader knows about our Rust peripherals.
- **No Bash Rule**: Bash is strictly relegated to simple CI aliases.

## Consequences
- **Positive**: 100% type-safe build scripts. Unified error handling and cross-platform reliability.
- **Negative**: Slightly increases the initial bootstrap time, as the build tooling itself must be compiled before the main project can be built.