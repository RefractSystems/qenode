# Container Developer Guide

Practical reference for building, testing, and debugging the virtmcu Docker images.

## Stage overview

```
debian:trixie-slim
        │
     [base]          vscode user (UID 1000), zsh + oh-my-zsh, GitHub CLI, uv, common tools
        │
  [toolchain]        build deps, ARM GNU Toolchain binary, Python pin (uv), CMake, FlatBuffers
        │
        ├── [devenv]    Rust + Node.js + Claude Code + Gemini CLI  ← devcontainer target
        ├── [builder]   QEMU compile + zenoh-c build               ← CI / smoke tests
        └── [runtime]   lean: QEMU binaries + Python tooling       ← production image
```

`builder` and `runtime` are not required for local development. `devenv` is the daily driver.

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
docker run --rm -it --user vscode virtmcu-devenv:dev zsh
```

All versions are read from the `VERSIONS` file automatically — no manual `--build-arg` needed.

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
docker run --rm virtmcu-base:dev bash -c "
    id vscode
    test -d /home/vscode/.oh-my-zsh && echo 'oh-my-zsh: ok'
    sudo -n true && echo 'sudo: ok'
    zsh --version
    uv --version
"

# toolchain
docker run --rm virtmcu-toolchain:dev bash -c "
    arm-none-eabi-gcc --version | head -1
    uv run --python 3.13 python --version
    cmake --version | head -1
    flatc --version
"

# devenv (as vscode — claude lives in ~/.local/bin)
docker run --rm --user vscode virtmcu-devenv:dev bash -c "
    node --version && npm --version
    claude --version
    cargo --version && rustc --version
    arm-none-eabi-gcc --version | head -1
"

# builder
docker run --rm virtmcu-builder:dev bash -c "
    qemu-system-arm --version
    ls \${QEMU_MODULE_DIR}/*.so | head -5
"

# runtime
docker run --rm virtmcu-runtime:dev bash -c "
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
| `arm-none-eabi-gcc: not found` after build | ARM toolchain download URL changed | Check current URL format at [ARM releases](https://developer.arm.com/downloads/-/arm-gnu-toolchain-downloads), update `ARM_TOOLCHAIN_VERSION` in `VERSIONS` |
| `flatc: error while loading shared libraries: libstdc++.so.6` | `libstdc++6` missing from image | It's explicitly listed in the `toolchain` apt block; verify it wasn't removed |
| oh-my-zsh install hangs | GitHub connectivity issue at build time | `docker build` with `--network=host`; or add `--no-cache` to skip the cached layer |
| `claude --version` fails in smoke test | Claude Code PATH not set | Must run as `--user vscode` or confirm `ENV PATH="/home/vscode/.local/bin:${PATH}"` is in Dockerfile |
| `NodeSource setup_N.x` fails | NodeSource added codename support for the current Debian release after this was written | The Dockerfile uses a direct binary download from nodejs.org — no codename dependency. If failing, check `NODE_VERSION` in `VERSIONS` and verify the nodejs.org dist URL |
| `uv python install` fails | Network issue fetching python-build-standalone | Retry; uv downloads OS-agnostic glibc binaries, not distro-specific packages |

---

## Inspecting a running devcontainer

When the devcontainer is open in VS Code, you can run these from the integrated terminal:

```bash
# Confirm you are the right user and shell
id                          # uid=1000(vscode)
echo $SHELL                 # /usr/bin/zsh
echo $0                     # zsh

# Confirm versions match VERSIONS file
arm-none-eabi-gcc --version | head -1
uv run python --version
cmake --version | head -1
claude --version
node --version

# Confirm the workspace virtual environment is active
which python                # should be /workspace/.venv/bin/python
python -c "import zenoh; print(zenoh.__version__)"
```

---

## Version management

All dependency versions live in one place: the `VERSIONS` file at the repo root.

**To bump a version:**

```bash
# 1. Edit VERSIONS
vim VERSIONS

# 2. Propagate to Dockerfile, pyproject.toml, requirements.txt, ci.yml, Cargo.toml
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
| `.github/workflows/ci.yml` | `PYTHON_VERSION` env block |
| `pyproject.toml` | `eclipse-zenoh`, `flatbuffers` |
| `requirements.txt` | `eclipse-zenoh`, `flatbuffers` |
| `tools/zenoh_coordinator/Cargo.toml` | `zenoh` crate version |
| `worlds/pendulum.yml` | inline `uv pip install eclipse-zenoh==` |

`check-versions` is a read-only enforcer run in the CI lint tier. It fails if any of the above are out of sync with `VERSIONS`, with a message pointing to `make sync-versions`.

---

## Upgrading the Debian base

The base image codename is the only thing needed to change:

```bash
# VERSIONS
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
