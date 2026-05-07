# VirtMCU Testing & Linting Infrastructure

This directory contains the core orchestration scripts for testing and linting the VirtMCU framework. 

These scripts are designed to be **location-agnostic** and can be reused by downstream projects (e.g., Firmware Studio) that include VirtMCU as a Git submodule. They automatically discover the VirtMCU root and apply the framework's strict Enterprise/SOTA quality standards to either the VirtMCU codebase itself or the parent repository.

## Key Scripts

### `run-lint.sh`
The unified lint runner. It executes all static analysis checks (Python, Rust, Shell, YAML, C, spelling, and version synchronization).
*   **Usage (Local):** `./scripts/testing/run-lint.sh`
*   **Usage (Downstream):** `./third_party/virtmcu/scripts/testing/run-lint.sh .` (Pass the target directory as an argument). When run against an external directory, it automatically applies the `--force-all` flag to enforce VirtMCU's banned patterns across the entire target repository.

### `run-unit*.sh`
Scripts for executing Rust unit tests under various conditions:
*   `run-unit.sh`: Standard cargo test runner.
*   `run-unit-miri.sh`: Runs tests under Miri to detect Undefined Behavior (UB) and memory leaks.
*   `run-unit-coverage.sh`: Generates an HTML coverage report for unit tests.

### `run-integration*.sh`
Scripts for executing the Python-based integration and system test suite using `pytest`:
*   `run-integration.sh`: Standard integration test runner. Handles QEMU compilation checks and artifact resolution.
*   `run-integration-asan.sh`: Runs the integration suite against QEMU built with AddressSanitizer (ASan) to detect memory corruption during simulated runs. Scales timeouts automatically.
*   `run-integration-coverage.sh`: Measures Python code coverage during integration tests.

### `run-peripheral-coverage.sh`
Generates coverage reports specifically for the Rust QOM peripheral plugins during integration test runs.

## Downstream Integration Guide

If you are using VirtMCU as a submodule, you can hook these scripts directly into your CI or Makefiles. For example:

```makefile
# In your parent project's Makefile
VIRMCU_DIR := third_party/virtmcu

.PHONY: lint
lint:
	@$(VIRMCU_DIR)/scripts/testing/run-lint.sh .
```

The scripts rely on `uv` for Python dependency management and expect a standard `cargo` toolchain for Rust.
