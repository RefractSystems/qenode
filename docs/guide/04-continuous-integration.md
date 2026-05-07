# Continuous Integration & Delivery

## The Reliability Engine

VirtMCU's CI/CD pipeline is designed for high-throughput, deterministic verification across multiple languages and architectures. By enforcing strict "Gates" at every stage—from local commits to multi-node smoke tests—we ensure that the `main` branch remains a stable, production-ready baseline.

---

## 1. Local CI Gates

Every level of our pipeline is reproducible locally. We do not rely on "magic" GitHub Actions; if a test fails in CI, it can be reproduced and debugged in your local environment.

### Level 0: Git Hooks
*   **Command**: `make install-git-hooks`
*   **Enforcement**: 
    *   **Pre-commit**: Runs `make dev-lint`. Prevents committing broken formatting or lint errors.
    *   **Pre-push**: Runs `make dev-unit`. Ensures all logic tests pass before code leaves your machine.
    *   This is the fastest feedback loop and is **strongly recommended** for all contributors.

### Level 1: `make dev-check`
*   **Purpose**: The "Fast Path" developer check.
*   **Mechanism**: Runs `dev-lint` and `dev-unit` natively in your current environment. Use this frequently during iteration.

### Level 2: `make ci-local`
*   **Purpose**: The "Safe Path" before pushing.
*   **Mechanism**: Executed inside the **isolated `devenv` Docker container**. This guarantees 1:1 parity with GitHub's Tier 1 checks. It mounts a persistent `.ci-target/` directory to ensure Rust builds remain fast across runs.

### Level 3: `make ci-full`
*   **Purpose**: Authoritative parity with the cloud.
*   **Mechanism**: Executes the full suite, including `ci-asan`/`ci-miri` passes and execution of all smoke test domains inside the isolated CI images.

---

## 2. Build & Test Target Parity

To ensure seamless transitions between local development and CI troubleshooting, VirtMCU maintains strict 1:1 parity between `dev-` (local) and `ci-` (containerized) targets.

| Domain | Local (`dev-`) | Container (`ci-`) | GitHub CI Equivalent |
| :--- | :--- | :--- | :--- |
| **All-in-one** | `make dev-check` | `make ci-local` | `tier-checks` |
| **Linting** | `make dev-lint` | `make ci-lint` | `tier-checks` (lint) |
| **Unit Tests** | `make dev-unit` | `make ci-unit` | `tier-checks` (unit) |
| **Miri (UB)** | `make dev-unit-miri` | `make ci-unit-miri` | `unit-miri` |
| **Integration** | `make dev-integration` | `make ci-integration` | `integration` |
| **ASan/UBSan** | `make dev-integration-asan` | `make ci-integration-asan` | `integration` (asan) |
| **Coverage** | `make dev-unit-coverage` | `make ci-unit-coverage` | `unit-coverage` |

### SOTA Developer Pro-Tips
*   **Persistent Caching**: Containerized targets (`ci-*`) now mount host directories (`.ci-target/`, `.cargo-cache/`) to avoid full rebuilds. If you experience mysterious build errors, run `make distclean` to wipe these caches.
*   **Dynamic Identity**: All local Docker runs automatically map your host `UID` and `GID` (via `HOST_UID`/`HOST_GID` environment variables). This prevents the common "root-owned files in workspace" bug.
*   **Targeted Integration**: Use `make dev-integration DOMAIN=boot_arm` to run only a specific test domain and save time.
*   **Fast Setup**: New to the project? Run `make setup-dev` to handle dependency installation, version synchronization, and the initial build in one command.

---

## 3. Docker Image Hierarchy

VirtMCU uses a multi-stage Docker strategy to optimize build times and minimize production image size.

1.  **`base`**: Debian slim + standard utilities.
2.  **`toolchain`**: Adds ARM/RISC-V compilers, Python, and CMake.
3.  **`devenv`**: Adds Rust, Node.js, and protocol schemas. Used for checks.
4.  **`builder`**: Compiles the patched QEMU core and all `.so` plugins. 
5.  **`devenv`**: The developer image (Base + pre-built QEMU from Builder).
6.  **`runtime`**: A lean production image containing only QEMU and Python orchestration tools.

---

## 3. Cache Architecture

To avoid the 40-minute QEMU compilation on every run, we use a three-layer cache:

1.  **Registry Cache (Primary)**: PRs and Main write intermediate layers to GHCR.
2.  **GHA Cache (Fallback)**: Per-stage scopes (e.g., `VirtMCU-builder-amd64`) provide a secondary speedup.
3.  **Layer Reuse**: The `qemu-builder` layer is only invalidated if `patches/`, `QEMU_VERSION`, or build flags change. Modifying Rust `hw/` sources only rebuilds the final plugin layer.

---

## 4. Version Management

All dependency versions (QEMU, Zenoh, compilers, Python) are centralized in a single source of truth: the **`BUILD_DEPS`** file at the repository root.

**To bump a version**:
1.  Edit `BUILD_DEPS`.
2.  Run `make sync-versions` to propagate the change to Dockerfiles, `pyproject.toml`, and GitHub workflows.
3.  Run `make check-versions` (enforced in CI lint) to verify consistency.

---

## 5. Troubleshooting CI Failures

| Symptom | Cause | Action |
|---|---|---|
| `CLOCK STALL` | ASan overhead or deadlock | Check QEMU stderr; system scales to 300s timeout under ASan. |
| `FFI Layout Mismatch` | C/Rust struct drift | Run `scripts/check-ffi.py --fix` and commit the updated offsets. |
| `can't find crate` | Cargo cache corruption | Run `docker volume rm ci-cargo-registry`. |
| `SIGSEGV` in plugin | Unmangled symbols | Ensure FFI hooks are wrapped in `VirtMCU_export!`. |

---

## 6. Testing Dockerfile Changes Locally

When making changes to `docker/Dockerfile`, you should verify them locally before pushing to GitHub to avoid breaking the CI pipeline (such as the `EOFError` crashes caused by missing configure flags).

1.  **Syntax Check**: Run `make dev-lint-docker` to use `hadolint` for basic syntax and best-practice checks.
2.  **Version Drift Check**: Run `make check-versions` to ensure all `ARG` versions in the Dockerfile match the single source of truth in `BUILD_DEPS`.
3.  **Fast Smoke Test**: Run `make docker-dev`. This builds the `base`, `toolchain`, and `devenv` stages and executes bash smoke tests to ensure essential tools (compilers, python, etc.) are actually installed and functional. This is much faster than a full build.
4.  **Full Parity Build**: If you touch critical QEMU flags or SystemC dependencies, run `make ci-full` to execute the full test matrix inside the newly built Docker image.
