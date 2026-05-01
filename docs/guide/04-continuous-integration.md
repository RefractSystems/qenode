# Chapter 4: Continuous Integration & Delivery

## The Reliability Engine

VirtMCU's CI/CD pipeline is designed for high-throughput, deterministic verification across multiple languages and architectures. By enforcing strict "Gates" at every stage—from local commits to multi-node smoke tests—we ensure that the `main` branch remains a stable, production-ready baseline.

---

## 1. Local CI Gates

Every level of our pipeline is reproducible locally. We do not rely on "magic" GitHub Actions; if a test fails in CI, it can be reproduced and debugged in your DevContainer.

### Level 0: Git Hooks
*   **Command**: `make install-hooks`
*   **Enforcement**: Runs `make lint` and `make test-unit` before every commit. This is the fastest feedback loop.

### Level 1: `make ci-local`
*   **Purpose**: Simulates the standard GitHub Tier 1 checks.
*   **Mechanism**: Runs `lint`, `build-tools`, and `test-unit` inside the `devenv-base` Docker image. Use this before opening a Pull Request.

### Level 2: `make ci-full`
*   **Purpose**: Authoritative parity with the cloud.
*   **Mechanism**: Executes the full suite, including ASan/Miri passes and sequential execution of all 20+ smoke test domains inside the `builder` image.

---

## 2. Docker Image Hierarchy

VirtMCU uses a multi-stage Docker strategy to optimize build times and minimize production image size.

1.  **`base`**: Debian slim + standard utilities.
2.  **`toolchain`**: Adds ARM/RISC-V compilers, Python, and CMake.
3.  **`devenv-base`**: Adds Rust, Node.js, and protocol schemas. Used for Tier 1 checks.
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
