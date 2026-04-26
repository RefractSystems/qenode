# Container Developer Guide

Practical reference for building, testing, and debugging the virtmcu Docker images.

## Quick Setup (The Happy Path)

To get started with zero friction:
1. **Clone via HTTPS:** `git clone https://github.com/RefractSystems/virtmcu.git`
2. **Open in VS Code:** Open the cloned folder in VS Code and click "Reopen in Container".
3. **Verify:** Once loaded, run `make test-unit` in the terminal. Everything should pass.

> **Note:** If you accidentally cloned via SSH, the DevContainer will automatically detect this and switch your remote to HTTPS on startup to ensure a stable git experience.

## FAQ: DevContainer "Magic Tricks" & Assumptions

We employ several automated tricks to make local development fast, parallel-safe, and identical to CI. Here is what is happening under the hood:

### Q: Why did my git remote change from SSH to HTTPS?
**A:** Docker for Mac/Windows frequently breaks SSH agent socket forwarding when the host machine goes to sleep or Docker Desktop restarts. By switching you to HTTPS, we leverage VS Code's built-in Git Credential Helper, which is rock-solid and survives sleep cycles. (See `.devcontainer/post-create.sh`).

### Q: How does parallel testing (`pytest -n auto`) work without port collisions?
**A:** Tests requiring a Zenoh router don't use fixed ports. Instead, `tests/conftest.py` invokes `scripts/get-free-port.py` to dynamically assign an available, ephemeral port for each test worker, preventing "Address already in use" errors.

### Q: Is `make clean-sim` safe to run if another developer/agent is testing concurrently?
**A:** Yes. `scripts/cleanup-sim.sh` is heavily optimized to be **Workspace-Scoped**. It inspects `/proc/<pid>/cwd` and `/proc/<pid>/cmdline` to ensure it *only* kills orphaned processes that originated from your specific cloned directory, leaving other workspaces entirely untouched.

### Q: What is the "FFI Gate" and why do I need to run `make check-ffi`?
**A:** To prevent cryptic segmentation faults, our Rust models and QEMU's C code must perfectly agree on memory layouts. The FFI Gate (`scripts/check-ffi.py`) uses `pahole` via `scripts/probe-qemu.py` to extract the *exact* byte offsets from the compiled QEMU binary and validates them against the Rust `assert!` statements.

---

> **CI failing?** See [CI_GUIDE.md](CI_GUIDE.md) — it maps every CI job to its local
> equivalent command and lists common failure patterns and fixes.

## Stage overview

```
debian:trixie-slim
        │
     [base]          vscode user (UID 1000), zsh + oh-my-zsh, GitHub CLI, uv, common tools
        │
  [toolchain]        build deps, ARM GNU Toolchain binary, Python pin (uv), CMake (lean)
        │
        ├── [devenv-base] Rust + Node.js + AI CLIs                 ← CI lint/unit tests
        │     │
        │     └── [devenv]  + pre-built QEMU binaries              ← devcontainer target
        │
        └── [simulation-toolchain]  + flatcc, zenoh-c              ← Intermediate sim layer
              │
              ├── [builder]   QEMU compile + plugins               ← CI / smoke tests
              └── [runtime]   lean: QEMU binaries + Python tooling ← production image
```

`builder` and `runtime` are not required for local development. `devenv` is the daily driver for VS Code, while `devenv-base` is used for ultra-fast local and CI lints without requiring a QEMU build. The `simulation-toolchain` decouples heavy C-level simulation dependencies (like `flatcc`) from the fast linting track.

---

## Why `/opt/virtmcu`? (Pre-baked vs. Local Builds)

A common point of confusion is why the container uses pre-built QEMU and Zenoh binaries located in `/opt/virtmcu` instead of compiling them directly inside the `/workspace` folder.

There are three main architectural reasons for this design:

1. **The Bind Mount Problem:** In a DevContainer, your host machine's directory is **bind-mounted** over the `/workspace` directory inside the container. If the `devenv` Docker image baked the compiled QEMU and Zenoh binaries into `/workspace`, the moment the container started, your host machine's folder would completely overwrite and hide them. To survive the bind mount, pre-baked dependencies *must* live outside the workspace (like in `/opt/`).
2. **Build Time & Velocity:** Compiling QEMU from scratch takes 15–40 minutes. By pre-compiling the emulator core and core dependencies (Zenoh-C, flatcc) during the Docker image build process, the DevContainer boots in seconds. The virtmcu project uses dynamic shared libraries (`.so` plugins); since the heavy emulator core is already built at `/opt/virtmcu/bin/qemu-system-arm`, you only need to incrementally compile your custom Rust/C peripherals in the `hw/` directory (which takes 2–3 seconds).
3. **Environment Parity:** `/opt/virtmcu` represents the immutable, known-good baseline provided by the CI environment. It guarantees that if a test passes locally against `/opt/virtmcu`, it will pass on GitHub Actions.

### The Escape Hatch: Modifying Core Dependencies

You are never locked out of modifying the core. If you decide you need to fix a header, edit QEMU's C code, or rebuild a core dependency, you can use the "escape hatch":

```bash
make setup-initial --force
```

**Here is what that does:**
1. It downloads the raw source code into `/workspace/third_party/` (inside your repo).
2. It compiles everything locally right there in your workspace.
3. The project's run scripts (`scripts/run.sh`) and CMake/Meson files are hardcoded to **look in your `third_party/` folder FIRST.**

If you have a local build in `third_party/`, the system completely ignores `/opt/virtmcu`. You get the fast, read-only baseline by default, but you can "eject" into a fully mutable local build the moment you need to hack on the core.

---

## Quick start

```bash
# Build base → toolchain → devenv with smoke tests (first run ~10 min)
make docker-dev

# Full pipeline including QEMU compile (~40 min first run, cached after)
make docker-all

# Override local image tag
IMAGE_TAG=my-branch make docker-dev

# Open an interactive shell in the devenv image
docker run --rm -it --user vscode ghcr.io/refractsystems/virtmcu/devenv:dev-amd64 zsh
```

All versions are read from the `BUILD_DEPS` file automatically — no manual `--build-arg` needed.

---

## Building a single stage

Use the individual targets when a specific stage is failing and you want a fast feedback loop:

```bash
make docker-base        # ~2 min — stops after base, no smoke test
make docker-toolchain   # ~8 min — stops after toolchain, no smoke test
make docker-devenv      # stops after devenv
make docker-builder     # ~40 min — QEMU full compile
make docker-runtime     # runtime only
```

These build the target stage plus all its dependencies (Docker cache applies). No smoke test is run, so you can inspect the image directly after a failure.

---

## Smoke tests

`make docker-dev` and `make docker-all` run smoke tests automatically after each stage. To run a smoke test manually on an already-built image:

```bash
# base
docker run --rm ghcr.io/refractsystems/virtmcu/base:dev-amd64 bash -c "
    id vscode
    test -d /home/vscode/.oh-my-zsh && echo 'oh-my-zsh: ok'
    sudo -n true && echo 'sudo: ok'
    zsh --version
    uv --version
"

# toolchain
docker run --rm ghcr.io/refractsystems/virtmcu/toolchain:dev-amd64 bash -c "
    arm-none-eabi-gcc --version | head -1
    uv run --python 3.13 python --version
    cmake --version | head -1
    flatc --version
"

# devenv-base (as vscode)
docker run --rm --user vscode ghcr.io/refractsystems/virtmcu/devenv-base:dev-amd64 bash -c "
    node --version && npm --version
    gemini --version || true # gemini cli might be aliased, but test npm global install
    cargo --version && rustc --version
    arm-none-eabi-gcc --version | head -1
"

# devenv (adds QEMU)
docker run --rm --user vscode ghcr.io/refractsystems/virtmcu/devenv:dev-amd64 bash -c "
    qemu-system-arm --version
"

# builder
docker run --rm ghcr.io/refractsystems/virtmcu/builder:dev-amd64 bash -c "
    qemu-system-arm --version
    ls \${QEMU_MODULE_DIR}/*.so | head -5
"

# runtime
docker run --rm ghcr.io/refractsystems/virtmcu/runtime:dev-amd64 bash -c "
    qemu-system-arm --version
    python3 -c 'import zenoh; print(zenoh.__version__)'
    python3 -c 'import flatbuffers; print(flatbuffers.__version__)'
"
```

---

## Debugging a failed build

**Strategy: build the failing stage alone, then inspect interactively.**

If a stage fails partway through, find the last successful layer ID from the build output and run a shell in it:

```bash
# 1. Run the failing build with plain output to see layer SHAs
docker build --target toolchain --progress=plain \
  --build-arg DEBIAN_CODENAME=trixie \
  ... \
  -f docker/Dockerfile . 2>&1 | tee /tmp/build.log

# 2. Find the last successful layer SHA in the log, then:
docker run --rm -it <last-good-sha> bash

# 3. Reproduce the failing RUN command manually inside the container
```

**Common failures and fixes:**

| Symptom | Likely cause | Fix |
|---|---|---|
| `arm-none-eabi-gcc: not found` after build | ARM toolchain download URL changed | Check current URL format at [ARM releases](https://developer.arm.com/downloads/-/arm-gnu-toolchain-downloads), update `ARM_TOOLCHAIN_VERSION` in `BUILD_DEPS` |
| `flatc: error while loading shared libraries: libstdc++.so.6` | `libstdc++6` missing from image | It's explicitly listed in the `toolchain` apt block; verify it wasn't removed |
| oh-my-zsh install hangs | GitHub connectivity issue at build time | `docker build` with `--network=host`; or add `--no-cache` to skip the cached layer |
| `gemini --version` fails in smoke test | Gemini CLI not installed globally | Verify `npm install -g @google/gemini-cli@latest` is in Dockerfile |
| `NodeSource setup_N.x` fails | NodeSource added codename support for the current Debian release after this was written | The Dockerfile uses a direct binary download from nodejs.org — no codename dependency. If failing, check `NODE_VERSION` in `BUILD_DEPS` and verify the nodejs.org dist URL |
| `uv python install` fails | Network issue fetching python-build-standalone | Retry; uv downloads OS-agnostic glibc binaries, not distro-specific packages |

---


## Git Authentication and "Self-Healing" Remotes

To ensure a stable and "bulletproof" development environment, we prefer **HTTPS with the Git Credential Helper** over SSH. This avoids common issues with broken SSH agent sockets on macOS/Windows after sleep cycles.

### The Recommended Path: HTTPS
When using DevContainers, always clone using the HTTPS URL:
\`\`\`bash
git clone https://github.com/RefractSystems/virtmcu.git
\`\`\`
VS Code automatically bridges its internal **Git Credential Helper** into the container. This provides a "zero-setup" experience that is immune to host network drops or sleep/wake cycles.

### Automated Self-Healing
If you previously cloned via SSH (\`git@github.com:...\`), the devcontainer will **automatically detect this** and switch your remote to HTTPS on the first launch. This ensures that features like \`git push\` work out of the box without manual SSH agent troubleshooting.

### Troubleshooting: GitHub CLI (gh)
The GitHub CLI (\`gh\`) is pre-installed. If it asks for authentication, it is best to run:
\`\`\`bash
gh auth login
\`\`\`
Inside the container, this will use a web-based flow. VS Code's built-in "Terminal Authentication" can sometimes interfere; if you see persistent errors, you can disable it in VS Code Settings: \`git.terminalAuthentication: false\`.


## Inspecting a running devcontainer

When the devcontainer is open in VS Code, you can run these from the integrated terminal:

```bash
# Confirm you are the right user and shell
id                          # uid=1000(vscode)
echo $SHELL                 # /usr/bin/zsh
echo $0                     # zsh

# Confirm versions match BUILD_DEPS file
arm-none-eabi-gcc --version | head -1
uv run python --version
cmake --version | head -1
gemini --version
node --version

# Confirm the workspace virtual environment is active
which python                # should be /workspace/.venv/bin/python
python -c "import zenoh; print(zenoh.__version__)"
```

---

## Version management

All dependency versions live in one place: the `BUILD_DEPS` file at the repo root.

**To bump a version:**

```bash
# 1. Edit BUILD_DEPS
vim BUILD_DEPS

# 2. Propagate to Dockerfile, pyproject.toml, requirements.txt, ci-pr.yml, ci-main.yml, Cargo.toml
make sync-versions

# 3. Verify everything is consistent (also runs in CI lint tier)
make check-versions

# 4. Rebuild and verify
make docker-dev
```

**What `sync-versions` touches:**

| File | Keys synced |
|---|---|
| `docker/Dockerfile` | All ARG defaults |
| `.github/workflows/ci-pr.yml`, `ci-main.yml` | `PYTHON_VERSION` env block |
| `pyproject.toml` | `eclipse-zenoh`, `flatbuffers` |
| `requirements.txt` | `eclipse-zenoh`, `flatbuffers` |
| `tools/zenoh_coordinator/Cargo.toml` | `zenoh` crate version |
| `Cargo.toml` | `zenoh`, `flatbuffers` |
| `worlds/pendulum.yml` | inline `uv pip install eclipse-zenoh==` |

`check-versions` is a read-only enforcer run in the CI lint tier. It fails if any of the above are out of sync with `BUILD_DEPS`, with a message pointing to `make sync-versions`.

---

## Upgrading the Debian base

The base image codename is the only thing needed to change:

```bash
# BUILD_DEPS
DEBIAN_CODENAME=forky   # was: trixie

make sync-versions
make docker-dev         # validates the new base builds and smoke-tests cleanly
```

**Before bumping the codename, verify these third-party dependencies support the new release:**

```bash
# 1. Node.js binary download (nodejs.org/dist — no codename dependency, always fine)
curl -fsSI "https://nodejs.org/dist/latest-v${NODE_VERSION}.x/SHASUMS256.txt"

# 2. GitHub CLI apt repo (uses 'stable main' channel — no codename, always fine)
# No check needed.

# 3. ARM GNU Toolchain (prebuilt binary — no OS dependency, always fine)
# No check needed.

# 4. Python via uv (python-build-standalone glibc binary — no OS dependency)
# No check needed.

# 5. Check that apt packages still exist under the new codename
docker run --rm debian:forky-slim apt-get update -qq \
  && apt-get install -y --dry-run \
     build-essential meson libslirp-dev gcc-riscv64-linux-gnu b4 lcov gcovr patchelf
```

If step 5 flags any missing packages, find the new package name in the Debian package tracker and update the `toolchain` apt block in `docker/Dockerfile`.

---

## Multi-arch builds

The CI builds native AMD64 and ARM64 images in parallel using GitHub's ARM runners (no QEMU emulation for the compile steps). Locally you can target a specific arch:

```bash
docker buildx build --platform linux/amd64 --target devenv ...
docker buildx build --platform linux/arm64 --target devenv ...
```

The ARM GNU Toolchain download uses `TARGETARCH` to select the correct host binary:
- `amd64` → `x86_64-arm-none-eabi`
- `arm64` → `aarch64-arm-none-eabi`

Both are bare-metal cross-compilers producing the same ARM firmware output regardless of host architecture.
