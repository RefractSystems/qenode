# Contributing to virtmcu

## Prerequisites

| Tool | Version | Notes |
|------|---------|-------|
| Git | Any | |
| Python | ≥ 3.11 | For repl2qemu, testing |
| GCC or Clang | Recent | C11 |
| Ninja | ≥ 1.10 | QEMU build |
| Meson | ≥ 1.0 | QEMU build |
| `dtc` | Any | Device Tree Compiler |
| `b4` | ≥ 0.14 | Fetching QEMU patch series |
| `pkg-config` | Any | |

**Platform**: macOS and Linux are both supported for development (Phases 1–3).
For Phase 4+ (TCG plugins), use Docker — macOS has a conflict between
`--enable-modules` and `--enable-plugins` (QEMU GitLab #516).
Windows is not supported (QEMU module loading is unavailable on Windows).

### macOS & Windows

> **⚠️ Mandatory Devcontainer:** virtmcu requires a Linux environment. macOS and Windows developers **MUST** use the provided Devcontainer. Bare-metal development on these platforms is not supported.

### Linux (Debian / Ubuntu)

```bash
sudo apt install build-essential libglib2.0-dev ninja-build \
                 device-tree-compiler flex bison libpixman-1-dev pkg-config \
                 b4

# Install uv for Python environment management
curl -LsSf https://astral.sh/uv/install.sh | sh
```

---

## First-Time Setup

### Recommended: Dev Container (VS Code)

Open the repo in VS Code and accept **"Reopen in Container"** when prompted.
The devcontainer automatically:
1. Builds the toolchain image (`docker/Dockerfile` `devenv` stage)
2. Initializes the QEMU submodule
3. Runs `make setup-initial` — patches and builds QEMU (~10 min, runs once)
4. Synchronizes the Python environment using `uv sync`
5. Activates the venv in every new terminal
6. Configures Git to use `gh auth git-credential` for push/pull.

**Authentication & Cloning (CRITICAL):**
To avoid painful SSH socket forwarding issues inside the devcontainer, **you must use the GitHub CLI (`gh`) for authentication and clone via HTTPS.**

```bash
# 1. Install GitHub CLI on your host machine
# Mac: brew install gh
# Linux: apt install gh

# 2. Authenticate to GitHub (Choose HTTPS when prompted for preferred protocol)
# Linux desktop users must use --insecure-storage
gh auth login

# 3. Clone this repo using HTTPS (NOT SSH)
git clone https://github.com/RefractSystems/virtmcu.git
cd virtmcu
```

The devcontainer securely maps your host's `~/.config/gh` directory so you don't need to re-authenticate inside the container. If you clone via HTTPS, your git pushes and pulls will work seamlessly without fighting SSH keys.

Nothing else is needed. Skip to [Development Workflow](#development-workflow).

### Manual Setup (macOS / Linux)

```bash
# 1. Clone this repo
git clone https://github.com/RefractSystems/virtmcu.git
cd virtmcu

# 2. Initialize the QEMU submodule
git submodule update --init --recursive

# 3. Build QEMU with all patches applied (~10 min first run)
make setup

# 4. Set up Python environment
make venv
source .venv/bin/activate

# 5. Smoke-test
make run
```

After `make setup`, QEMU lives in `third_party/qemu/build-virtmcu/install/` (or `build-virtmcu-asan/install/` if `VIRTMCU_USE_ASAN=1` is used).
`scripts/run.sh` is a wrapper that sets the module dir and launches
the right QEMU binary.

---

## Development Workflow

### Adding a New Peripheral

**For Rust Models (Preferred):**
1. Copy the `hw/rust/rust-dummy/` template to `hw/rust/<name>`.
2. Edit `src/lib.rs` for your `#[no_std]` Rust implementation.
3. Update `hw/meson.build` to compile and link your module.

**For C Models (Legacy/Bridge only):**
1. Copy `hw/rust/virtmcu-test-devices/dummy.c` (as a template) or use existing C models.
2. Add an entry to `hw/meson.build` following the existing pattern.

**For all models:**
4. Run `make build` — only changed files recompile.
5. Test:
   ```bash
   ./scripts/run.sh --dtb test/phase1/minimal.dtb \
                    -device <your-device-name> -nographic
   ```
6. Verify the type appears in `-device help` output.

### Changing QEMU Patches

Our patches live in `patches/`.  The applied patch branch in the QEMU tree
is `virtmcu-patches`.

```bash
# Make changes in third_party/qemu, then:
cd third_party/qemu
git add -p          # stage your changes
git commit -m "your patch description"

# Export the new patch:
cd <virtmcu-repo>
git -C third_party/qemu format-patch HEAD~1 -o patches/

# Or regenerate the full series:
git -C third_party/qemu format-patch <base-commit>..HEAD -o patches/
```

### Cleaning up Simulation Processes

During development, especially when debugging integration tests or dealing with unexpected timeouts, you might encounter multiple "stale" QEMU or Zenoh processes running in the background. These orphaned processes can hold onto ports, UNIX sockets, or CPU resources, causing subsequent test runs to fail mysteriously.

To ensure a completely clean environment:

```bash
make clean-sim
```

This command runs `scripts/cleanup-sim.sh`, which safely terminates all running instances of `qemu-system-arm`, `qemu-system-riscv`, `qemu-system-aarch64`, `zenoh_router`, and `zenoh_coordinator`. It also cleans up any residual temporary files (like `*.dtb` and `.sock`) left in `/tmp`.

*Note: The `make test-integration` target automatically runs this cleanup script before and after every test.*

### Python Tools (`tools/`)

```bash
source .venv/bin/activate
uv run python -m tools.repl2qemu path/to/board.repl --out-dtb board.dtb --print-cmd
uv run pytest tests/ -v
```

## CI and Pre-Push Validation

Before opening a PR or pushing code to `main`, you should run our local CI validation suite to catch linting, versioning, and compilation errors. Our CI pipeline is highly optimized using GHCR caching.

*   **Fast Pre-Push Check (~2 mins):** Run `make ci-local` to run all static analysis, version checks, and unit tests.
*   **Static Analyzers & Memory Sanitizers:** Run `make ci-miri` (for Rust Undefined Behavior) and `make ci-asan` (for Memory Sanitizers).
*   **Full Pipeline Validation (~40 mins cold, fast if cached):** Run `make ci-full` to execute the complete matrix of smoke tests exactly as they run on GitHub Actions inside the isolated builder container. This also includes the Miri and ASan checks. Passing this guarantees GitHub CI will pass.

**For a detailed breakdown of how our CI pipeline works, how it uses Docker layer caching via GHCR, and how to debug specific failures, please read the [CI/CD Guide](docs/CI_GUIDE.md).**

---

## Testing and Regression

virtmcu relies on automated testing to ensure new features (like parsing or new peripherals) don't break earlier architectural work. All tests must be properly documented.

**Parallel Execution Strict Rules:** Our test suites run in parallel by default (`-n auto`). When writing new tests, they **MUST** be designed "out of the box" for parallel execution isolation. Do not rely on fixed resources:
1. **NO Hardcoded Ports:** Never use fixed ports like `tcp/127.0.0.1:7447`. Inject dynamic ports using the `zenoh_router` fixture (in Python) or `scripts/get-free-port.py` (in Bash).
2. **NO Hardcoded Temporal Paths:** Never generate output files (`.dtb`, `.yaml`, `.cli`) directly in `workspace_root` or shared test directories. Always use `pytest`'s `tmp_path` fixture or `mktemp -d` in Bash.
3. **NO Random Value Collisions:** Do not use `random.randint()` for IDs or paths. Use deterministic uniqueness tied to the execution context (e.g., `os.getpid()` or `tmp_path`).
4. **NO Manual Process Management:** Do not spawn background daemons (like `cargo run ... zenoh_coordinator`) manually inside tests. Use the provided reusable `pytest` fixtures for clean, isolated orchestration.

We split testing into two categories:

### 1. Emulator-Level Smoke Tests
These are raw `bash` scripts combined with small Python scripts (using QMP) to verify the emulator works at a low level.
They are located in `test/phaseX/smoke_test.sh`.

**To run all integration smoke tests locally:**
The Makefile automatically handles building required test artifacts (like ELFs) and setting up the Python environment before running the tests.
```bash
make smoke-tests
```
*(Note: `make test-integration` is an exact alias for this command.)*

**Running in a Mirrored CI Environment (Docker):**
If a test passes locally but fails on CI, you can run the test inside the exact `virtmcu-builder` container used by GitHub Actions.
```bash
# 1. Build the builder image from scratch
docker build -t virtmcu-builder -f docker/Dockerfile --target builder .

# 2. Run a specific phase smoke test (e.g., Phase 1)
docker run --rm \
  -v "$(pwd):/workspace" \
  -w /workspace \
  -e PYTHONPATH=/workspace \
  virtmcu-builder \
  bash -c "make -C test/phase1 && bash test/phase1/smoke_test.sh"
```

### Debugging Failed Smoke Tests
*   **Inspect the Logs:** Many tests capture QEMU output to a log file (e.g., `smoke_test_output.log` in the test directory). Always read this file if the test fails.
*   **Run Interactively:** If a smoke test times out or fails, run the QEMU command interactively without the `-monitor none` or `-serial file:...` flags so you can see what QEMU prints to the terminal.
    ```bash
    ./scripts/run.sh --dtb test/phase1/minimal.dtb --kernel test/phase1/hello.elf -nographic
    ```
    *(To exit an interactive QEMU session, press `Ctrl+A` followed by `X`)*
*   **Add Debug Flags:** You can append `-d exec,cpu_reset` or `-trace "zenoh_*"` to the `run.sh` command to trace execution blocks and see exactly where the firmware or QEMU is hanging.

### 2. Python Unit & Automation Tests (Phase 4+)
For testing the `repl2qemu` parser and the Robot Framework QMP automation bridge, we use `pytest`.

**To run unit/automation tests:**
```bash
# Make sure your virtual environment is synchronized!
make test
```

When implementing a feature for a new Phase, you **MUST** provide a corresponding `smoke_test.sh` (or `pytest` suite for later phases) before submitting your PR. This prevents regressions.

---

## Testing Strategy (Bifurcated: Rust + Python)

We adhere to a strict **Bifurcated Testing Strategy** to maximize performance, safety, and orchestration reliability.

1.  **White-Box Internals (Rust `#[test]`)**
    *   **What goes here:** State machines, lock-free queue logic, protocol parsing, memory layout validation (via `bindgen`), and anything testing internal C/Rust FFI boundaries.
    *   **How:** Write native `#[test]` modules inside the `hw/rust/` crates. These tests should *not* boot QEMU. Mock the necessary FFI calls.
    *   **Why:** Rust's test runner is exceptionally fast, and it can catch concurrency or memory alignment issues at compile time.

2.  **Black-Box Orchestration (Python `pytest`)**
    *   **What goes here:** Multi-node integration tests, QMP interaction, UART verification, process management (QEMU + Zenoh + TimeAuthority), and end-to-end regression testing.
    *   **How:** Write structured tests using `pytest` and `asyncio` using our existing fixtures in `tools/testing/qmp_bridge.py` and `conftest.py`.
    *   **Why:** Python handles complex multi-process orchestration, asynchronous teardowns, and string matching much better than Rust or Bash.

3.  **Thin CI Wrappers (Bash)**
    *   **Rule:** Bash (`test/*/*.sh`) is for entry points *only* (to satisfy the `make test-integration` contract).
    *   **Never** write complex background process setup/teardown loops in Bash. Just call `pytest` or `cargo test`.

---

## AI-Assisted Workflows (Auto Green)

If you are using an AI agent (Claude Code, Gemini CLI), you can automate the process of fixing broken CI/CD builds. If a PR build fails:

1.  Open the workspace in your agent.
2.  Command: **"Fix CI and make it green"**.
3.  The agent will autonomously:
    *   Diagnose the failure using `gh`.
    *   Reproduce the failure locally.
    *   Align the local test environment if the failure only occurs in CI.
    *   Apply the fix and verify locally.
    *   Monitor the remote run until all checks are green.

---

## Branching and Commits

- Branch off `main`: `git checkout -b feature/<phase>-<short-desc>`
- Commit style: `scope: imperative description`
  - `hw/uart: add pl011 mmio read/write stubs`
  - `tools/repl2qemu: handle using keyword in parser`
  - `scripts: add --arch flag to run.sh`
- One logical change per commit.
- Keep C changes and build system changes in separate commits.

---

## Code Style

**Rust (Preferred)**:
- Follow standard Rust idioms.
- Run `cargo clippy` and `cargo fmt`.
- All `unsafe` blocks must have a `// SAFETY:` comment explaining why it is safe.
- Avoid external crates in the simulation hot-path unless they are `no_std` compatible or specifically approved.

**C**: Follow QEMU's coding style (largely Linux kernel style).
- `qemu/osdep.h` must be the first include in every `.c` file.
- Use `qemu_log_mask(LOG_UNIMP, ...)` for unimplemented register accesses.
- Use `DEFINE_TYPES()` + `TypeInfo[]`, not the older `type_register_static()`.

**Python**: PEP 8, `ruff` for linting.
```bash
ruff check tools/ tests/
```

---

## Project Context

virtmcu is developed alongside **FirmwareStudio** (separate upstream repo),
a digital twin environment where MuJoCo drives physical simulation and acts as the
**external time master** for QEMU. See `CLAUDE.md` for the full architectural picture,
and `PLAN.md` for the phased task checklist.
