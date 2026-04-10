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

### macOS (Homebrew)

```bash
brew install ninja meson dtc pkg-config glib pixman b4
```

### Linux (Debian / Ubuntu)

```bash
sudo apt install build-essential libglib2.0-dev ninja-build python3-venv \
                 device-tree-compiler flex bison libpixman-1-dev pkg-config \
                 b4
```

---

## First-Time Setup

### Recommended: Dev Container (VS Code)

Open the repo in VS Code and accept **"Reopen in Container"** when prompted.
The devcontainer automatically:
1. Builds the toolchain image (`docker/Dockerfile` `devenv` stage)
2. Initializes the QEMU submodule
3. Runs `make setup` — patches and builds QEMU (~10 min, runs once)
4. Creates the Python venv and installs dependencies
5. Activates the venv in every new terminal

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

After `make setup`, QEMU lives in `third_party/qemu/build-virtmcu/install/`.
`scripts/run.sh` is a wrapper that sets the module dir and launches
the right QEMU binary.

---

## Development Workflow

### Adding a New Peripheral

1. Copy `hw/dummy/dummy.c` to `hw/<name>/<name>.c`.
2. Rename all `DUMMY`/`dummy` occurrences to your device name.
3. Add an entry to `hw/meson.build` following the existing pattern.
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

### Python Tools (`tools/`)

```bash
source .venv/bin/activate
python -m tools.repl2qemu path/to/board.repl --out-dtb board.dtb --print-cmd
python -m pytest tests/ -v
```

---

## Testing and Regression

virtmcu relies on automated testing to ensure new features (like parsing or new peripherals) don't break earlier architectural work. All tests must be properly documented.

We split testing into two categories:

### 1. Emulator-Level Smoke Tests (Phases 1-3)
These are raw `bash` scripts combined with small Python scripts (using QMP) to verify the emulator works at a low level.
They are located in `test/phaseX/smoke_test.sh`.

**To run all integration smoke tests:**
```bash
make test-integration
```
*Note: This will execute every script sequentially. If a single script fails, the make command exits immediately.*

### 2. Python Unit & Automation Tests (Phase 4+)
For testing the `repl2qemu` parser and the Robot Framework QMP automation bridge, we use `pytest`.

**To run unit/automation tests:**
```bash
# Make sure your virtual environment is active!
make test
```

When implementing a feature for a new Phase, you **MUST** provide a corresponding `smoke_test.sh` (or `pytest` suite for later phases) before submitting your PR. This prevents regressions.

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
