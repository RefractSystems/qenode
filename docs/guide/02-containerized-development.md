# Chapter 2: Containerized Development

## The "It Works" Guarantee

VirtMCU is designed for a seamless, container-first development experience. By utilizing VS Code DevContainers, we eliminate the "host pollution" and toolchain drift that plagues multi-language embedded projects.

---

## 1. Quick Start (The Happy Path)

To set up a local development environment with zero manual configuration:
1.  **Clone**: `git clone https://github.com/RefractSystems/VirtMCU.git`
2.  **Open**: Launch the folder in VS Code.
3.  **Reopen**: Click **"Reopen in Container"** when prompted.
4.  **Verify**: Run `make test-unit` in the integrated terminal.

---

## 2. Architectural Guardrails

### Why `/opt/VirtMCU`?
The DevContainer provides pre-built QEMU and Zenoh binaries in `/opt/VirtMCU`. This design serves three purposes:
1.  **Speed**: Compiling QEMU takes ~40 minutes. Pre-baking it allows the container to boot in seconds.
2.  **Isolation**: Pre-built binaries live outside the bind-mounted `/workspace`, ensuring they aren't accidentally overwritten by host files.
3.  **Parity**: `/opt/VirtMCU` is identical to the environment used in CI, guaranteeing that "passing locally" means "passing globally."

### The "Escape Hatch"
If you need to modify the QEMU core or C-level dependencies:
```bash
make setup-initial --force
```
This downloads and compiles everything into `/workspace/third_party/`. The VirtMCU run scripts are hardcoded to prioritize `third_party/` over `/opt/VirtMCU` whenever local builds are present.

---

## 3. Development "Magic Tricks"

To keep development fast and parallel-safe, we employ several automated mechanisms:

### Self-Healing Remotes
The container automatically detects if you cloned via SSH and switches the remote to **HTTPS**. This leverages VS Code's robust Git Credential Helper, avoiding issues with broken SSH agent forwarding after host sleep cycles.

### Parallel-Safe Testing
`pytest -n auto` assigns ephemeral ports for every Zenoh router and QMP instance. This prevents "Address already in use" errors during parallel test execution.

### Workspace-Scoped Cleanup
`make clean-sim` (via `scripts/cleanup-sim.sh`) only kills orphaned processes that originated from your specific workspace directory. It is safe to run even if other developers are testing concurrently on the same host.

### The FFI Gate
To prevent segmentation faults across the C/Rust boundary, `make check-ffi` extracts exact byte offsets from the QEMU binary and validates them against Rust's memory layout assertions.
