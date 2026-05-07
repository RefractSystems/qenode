# VirtMCU Enterprise Linting Suite

This directory contains the custom static analysis tools and linting rules enforced across the VirtMCU framework. 

These scripts guarantee that all code—whether internal to VirtMCU or in a downstream parent project—adheres strictly to our Enterprise/SOTA quality standards, focusing on deterministic simulation, memory safety, and robust system design.

## Design for Downstream Reuse

All lint scripts in this directory are designed to be **location-agnostic** and highly configurable. Parent repositories (e.g., Firmware Studio) can execute these scripts against their own codebases to ensure architectural alignment with the VirtMCU simulation engine.

When invoked via the master runner (`../testing/run-lint.sh`), they automatically adjust their scoping to cover the target project appropriately.

## Key Lints

### Banned Patterns
Enforces the "Fail Fast / Crash Only" mandate and bans non-deterministic or unsafe APIs.
*   **`rust_banned_patterns.py`**: Scans Rust code for forbidden practices (e.g., raw `Mutex`, `thread::sleep`, non-deterministic RNGs, bounded spinloops). Supports `--force-all` to apply rules project-wide.
*   **`python_banned_patterns.py`**: Scans Python code for anti-patterns (e.g., `logger.warning`, raw `zenoh.open` in tests, hardcoded timeouts). Supports `--force-all`.

### Architectural Integrity
*   **`rust_static_state.py`**: Absolutely bans `static mut` and global state inside dynamic plugins (`.so` files) to prevent cross-contamination and Dynamic Shared Object (DSO) boundary violations.
*   **`rust_safe_serialization.py`**: Ensures binary compatibility by preventing raw struct serialization; mandates FlatBuffers or explicit endian-safe logic.
*   **`rust_magic_numbers.py`**: Prevents the use of undocumented magic numbers in hardware modeling.
*   **`environment_agnosticism.py`**: Bans absolute paths and user-specific directories (e.g., `/home/`, `/tmp/`) to ensure builds and tests run anywhere.

### Process & Dependency Management
*   **`beyonce_rule.py`**: ("If you liked it, you should have put a test on it") - Enforces that every code change is accompanied by a corresponding test change. Configurable via `--watch` and `--test-dir`.
*   **`dependency_pinning.py`**: Ensures all Python and Rust dependencies are strictly pinned to exact versions.
*   **`third_party_modifications.py`**: Prevents direct manual edits to submodules like QEMU; mandates that changes go through the `patches/` system.
*   **`lint_audit.py`**: Analyzes the usage of `virtmcu-allow` comment escapes, preventing them from creeping into production code undetected.

### Common Utilities
*   **`lint_utils.py`**: Provides shared path resolution, standard logging, and the core `iter_target_files` logic, which safely handles complex submodule and nested directory exclusions.

## Bypassing Lints (Last Resort)

If a pattern must be broken for a structurally unavoidable reason, you must use the standard `virtmcu-allow` escape format. **This is highly discouraged in production code.**

```rust
// virtmcu-allow: sleep reasoning="Waiting for external hardware reset line"
std::thread::sleep(Duration::from_millis(10));
```
