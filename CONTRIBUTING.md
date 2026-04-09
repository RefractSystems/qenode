# Contributing to qenode

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

```bash
# 1. Clone this repo
git clone https://github.com/<org>/qenode.git
cd qenode

# 2. Build QEMU with all patches applied (takes ~5 min)
make setup

# 3. Set up Python environment
make venv
source .venv/bin/activate

# 4. Smoke-test
make run
```

After `make setup`, QEMU lives in `ext/qemu/build-qenode/install/`.
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
   ./scripts/run.sh -M arm-generic-fdt -hw-dtb tests/phase1/minimal.dtb \
                    -device <your-device-name> -nographic
   ```
6. Verify the type appears in `-device help` output.

### Changing QEMU Patches

Our patches live in `patches/`.  The applied patch branch in the QEMU tree
is `qenode-patches`.

```bash
# Make changes in ext/qemu, then:
cd ext/qemu
git add -p          # stage your changes
git commit -m "your patch description"

# Export the new patch:
cd <qenode-repo>
git -C ext/qemu format-patch HEAD~1 -o patches/

# Or regenerate the full series:
git -C ext/qemu format-patch <base-commit>..HEAD -o patches/
```

### Python Tools (`tools/`)

```bash
source .venv/bin/activate
python -m tools.repl2qemu path/to/board.repl --out-dtb board.dtb --print-cmd
python -m pytest tests/ -v
```

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

qenode is developed alongside **FirmwareStudio** (separate upstream repo),
a digital twin environment where MuJoCo drives physical simulation and acts as the
**external time master** for QEMU. See `CLAUDE.md` for the full architectural picture,
and `PLAN.md` for the phased task checklist.
