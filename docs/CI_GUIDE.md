# CI/CD Guide

How to understand the CI pipeline, reproduce failures locally, and know when a
failure is your code vs. a flaky runner.

---

## Pipeline overview

```
push / PR
    │
    ├── [Tier 1 — always run in parallel, ~2 min]
    │       lint            ruff + shellcheck + hadolint
    │       check-versions  VERSIONS file vs Dockerfile/pyproject.toml/requirements.txt
    │       unit-tests      pytest (no QEMU, no Docker)
    │
    ├── [Tier 2 — needs Tier 1 to pass]
    │       build-qemu      Docker builder stage (QEMU compile, ~40 min cold / ~3 min cached)
    │
    ├── [Tier 3 — needs build-qemu]
    │       smoke-tests (matrix)   phases 1–3.5, 4, 8–12 inside builder container
    │       smoke-phase5            SystemC bridge
    │       smoke-phase6            Zenoh coordinator
    │       smoke-phase7            zenoh-clock (suspend + icount modes)
    │       smoke-phase16           IPS benchmark + determinism
    │       pytest-qmp              QMP integration suite (real QEMU instances)
    │       robot-tests             Robot Framework keyword suite
    │       verify-runtime          Builds the runtime image and validates binaries
    │
    └── [Tier 4 — needs all Tier 3]
            peripheral-coverage     C plugin gcovr report (re-runs key tests)
            firmware-coverage       Guest drcov coverage via TCG plugin
```

Tier 1 and Tier 2 are required gates — a failure in either blocks everything downstream.
Tier 3 and Tier 4 jobs run in parallel and fail independently.

---

## Reproducing failures locally

### Step 1 — identify the failing tier

Open the failing workflow run in GitHub. Jobs are colour-coded red. The job name maps
directly to the section headings below.

### Step 2 — run the local equivalent

```bash
# Tier 1 + Tier 2 (fast path, ~10-15 min, no QEMU build required)
make ci-local

# Tier 1 + Tier 2 + Tier 3 sample (Phase 1 smoke + unit tests inside builder)
make ci-full   # ~40-50 min first run; cached builder reuses Docker layers
```

`make ci-local` is the daily pre-push check. `make ci-full` is what to run when
you have made changes to `hw/`, `patches/`, or `scripts/setup-qemu.sh` — anything
that touches the QEMU compile.

---

## Tier 1 — Lint, check-versions, unit-tests

### `lint` job

CI runs three linters in sequence. The local equivalents:

| CI step | Local command | Notes |
|---|---|---|
| Ruff | `uv run ruff check tools/ tests/ patches/` | Auto-fix: `uv run ruff check --fix ...` |
| ShellCheck | `shellcheck scripts/*.sh` | Install: `brew install shellcheck` |
| Hadolint | `hadolint --ignore DL3008,DL3009,DL4006,SC2016,SC2015 docker/Dockerfile` | Install: `brew install hadolint` |

`make ci-local` runs all three. If shellcheck or hadolint are not installed it
prints a warning and skips — install them for full parity.

**Common `lint` failures:**

| Error | Fix |
|---|---|
| `E501 line too long` or similar ruff errors | `make fmt` auto-fixes most; remaining require manual edit |
| `SC2086: Double quote to prevent globbing` | Wrap the variable in `"${VAR}"` in the shell script |
| `DL3007: Using latest` | Pin the image tag in the Dockerfile |
| Any SC* rule from hadolint | Hadolint checks shell inside `RUN` blocks; fix the `RUN` command or add an inline `# hadolint ignore=SCxxxx` comment |

---

### `check-versions` job

Verifies that every entry in the `VERSIONS` file matches the corresponding ARG
default in `docker/Dockerfile`, pin in `pyproject.toml`, and pin in `requirements.txt`.

```bash
# Run locally — exact CI match
make check-versions
python3 scripts/check-versions.py   # same thing, bypasses make
```

**Common `check-versions` failures:**

| Error message | Fix |
|---|---|
| `VERSIONS[KEY] = X but Dockerfile ARG default = Y` | Run `make sync-versions` |
| `pyproject.toml has flatbuffers>=X (floor) but VERSIONS says X.Y.Z` | Run `make sync-versions` — it enforces exact pins in both files |
| `requirements.txt pin does not match VERSIONS` | Run `make sync-versions` |

**Rule:** edit `VERSIONS`, then `make sync-versions`, then `make check-versions`.
Never hand-edit the downstream files — the sync script owns them.

---

### `unit-tests` job

Runs the pytest suite that does not require QEMU or Docker. Needs
`device-tree-compiler` for the FDT emitter test.

```bash
# Exact CI command
uv run pytest \
  tests/repl2qemu/ \
  tests/test_yaml2qemu.py \
  tests/test_cli_generator.py \
  tests/test_fdt_emitter.py \
  -v --tb=short

# Install dtc if test_fdt_emitter.py fails with "dtc: command not found"
brew install dtc          # macOS
sudo apt-get install device-tree-compiler   # Ubuntu/Debian
```

**Common `unit-tests` failures:**

| Symptom | Fix |
|---|---|
| `dtc: command not found` | Install `device-tree-compiler` (see above) |
| Assertion error in `test_yaml2qemu.py` | The YAML → DTB emitter output changed; run the failing test with `-s` to see the diff |
| Import error | `uv sync` to refresh the virtual environment |

---

## Tier 2 — build-qemu

This job builds the Docker `builder` stage, which clones QEMU at `QEMU_REF`,
applies all patches from `patches/`, builds zenoh-c, then compiles QEMU itself.

First run: ~40 min. Subsequent runs with unchanged `patches/`, `hw/`,
`scripts/setup-qemu.sh`: ~3 min (GitHub's GHA Docker layer cache).

```bash
# Build the builder image locally (same layers, no GHA cache — uses local Docker cache)
make docker-builder

# Single-stage build with plain output for debugging layer failures
docker build --target builder --progress=plain \
  $(cat VERSIONS | grep -v '^#' | grep -v '^$' | sed 's/\(.*\)=\(.*\)/--build-arg \1=\2/' | tr '\n' ' ') \
  -f docker/Dockerfile . 2>&1 | tee /tmp/builder.log
```

**Common `build-qemu` failures:**

| Symptom | Likely cause | Fix |
|---|---|---|
| `arm-gnu-toolchain*.tar.xz: 404 Not Found` | ARM download URL changed | Check [ARM releases](https://developer.arm.com/downloads/-/arm-gnu-toolchain-downloads) for the current filename format; update `ARM_TOOLCHAIN_VERSION` in `VERSIONS` |
| `flatc: error while loading shared libraries: libstdc++.so.6` | `libstdc++6` accidentally removed from toolchain stage | Re-add it to the toolchain `apt-get install` block in `docker/Dockerfile` |
| Patch fails to apply | A patch in `patches/` no longer applies cleanly to the pinned QEMU ref | Run `scripts/setup-qemu.sh` locally with `QEMU_REF` set and inspect the reject files |
| zenoh-c cargo build OOM | CI runner ran out of memory during Rust compile | Re-run the job (transient); if persistent, reduce parallel jobs in the Cargo invocation |
| `E: Package 'xxx' has no installation candidate` | A package name changed in `trixie` | Find the new package name at [packages.debian.org](https://packages.debian.org) and update the Dockerfile |

---

## Tier 3 — Smoke tests

Each smoke test job:
1. Restores the builder image from the GHA Docker layer cache.
2. Mounts the workspace at `/workspace` inside the container.
3. Runs a `pre` command to build test artifacts (cross-compiled firmware).
4. Runs `bash test/phaseN/smoke_test.sh`.

### Running a specific phase smoke test locally

```bash
# Requires the builder image to be present — build it first if needed:
make docker-builder

# General pattern (replace N and <pre-command>):
docker run --rm \
  -v "$(pwd):/workspace" -w /workspace \
  -e PYTHONPATH=/workspace \
  -e VIRTMCU_STALL_TIMEOUT_MS=60000 \
  virtmcu-builder:dev \
  bash -c "<pre-command> && bash test/phaseN/smoke_test.sh"

# Phase 1 (ARM bare-metal boot, no Python needed)
docker run --rm \
  -v "$(pwd):/workspace" -w /workspace \
  -e VIRTMCU_STALL_TIMEOUT_MS=60000 \
  virtmcu-builder:dev \
  bash -c "make -C test/phase1 && bash test/phase1/smoke_test.sh"

# Phase 7 (zenoh-clock)
docker run --rm \
  -v "$(pwd):/workspace" -w /workspace \
  -e PYTHONPATH=/workspace \
  -e VIRTMCU_STALL_TIMEOUT_MS=60000 \
  virtmcu-builder:dev \
  bash -c "uv pip install --system --break-system-packages -r pyproject.toml && bash test/phase7/smoke_test.sh"

# Phase 8 (uv venv path)
docker run --rm \
  -v "$(pwd):/workspace" -w /workspace \
  -e PYTHONPATH=/workspace \
  -e VIRTMCU_STALL_TIMEOUT_MS=60000 \
  virtmcu-builder:dev \
  bash -c "make -C test/phase1 && make -C test/phase8 && uv sync && bash test/phase8/smoke_test.sh"
```

See `ci.yml` `smoke-tests` matrix for the exact `pre` command for each phase.

### `VIRTMCU_STALL_TIMEOUT_MS`

CI sets this to `60000` (60 s) because shared runners are slower than developer
machines. The default in the firmware is 5 s. If a smoke test passes locally but
times out in CI, it is a runner load issue — re-run the job. If it fails
consistently, the clock quantum is not advancing: check that the firmware is not
stuck in a tight polling loop (use ARM Generic Timer interrupts at 100 Hz instead).

### `pytest-qmp` job

```bash
# Requires builder image
docker run --rm \
  -v "$(pwd):/workspace" -w /workspace \
  -e PYTHONPATH=/workspace \
  -e VIRTMCU_STALL_TIMEOUT_MS=60000 \
  virtmcu-builder:dev \
  bash -c "
    make -C test/phase1
    make -C test/phase8
    uv pip install --system --break-system-packages -r pyproject.toml pytest-cov
    pytest tools/testing/test_qmp.py -v --tb=short
  "
```

### `robot-tests` job

```bash
docker run --rm \
  -v "$(pwd):/workspace" -w /workspace \
  -e PYTHONPATH=/workspace \
  virtmcu-builder:dev \
  bash -c "
    make -C test/phase1
    make -C test/phase8
    uv pip install --system --break-system-packages -r pyproject.toml
    robot --outputdir test-results/robot --loglevel INFO \
      tests/test_qmp_keywords.robot tests/test_interactive_echo.robot
  "
```

---

## Tier 4 — Coverage

Coverage jobs re-run a representative set of tests and collect gcov/drcov data.
They depend on all Tier 3 jobs passing. If only a coverage job is red:

1. The underlying test that the coverage job re-runs is likely flaky — check the
   Tier 3 job for the same phase to confirm.
2. The gcovr or drcov collection step failed — this is a tooling issue, not a
   firmware issue. Re-run the job.

Coverage failures do not block merges unless the repo has branch protection rules
requiring them. Check the PR's required status checks.

---

## Flaky vs. broken

| Pattern | Diagnosis | Action |
|---|---|---|
| Job passes on re-run without code changes | Flaky runner (OOM, network timeout, cache miss) | Re-run the failed job via GitHub UI |
| Job fails on every run, was green before your PR | Your change broke something | Run `make ci-local` and identify the failing check |
| Job has never been green on this branch | New test or check you introduced | Fix the test or the code it covers |
| `check-versions` fails after bumping `VERSIONS` | Forgot to run `make sync-versions` | `make sync-versions && git add -p && git commit --amend` |
| Builder cache miss causes timeout | Upstream QEMU or zenoh-c fetch is slow | Re-run; if persistent, check network connectivity from GHA runners |

---

## Local parity gaps

These CI checks cannot be run locally without additional setup:

| CI check | Gap | Install |
|---|---|---|
| ShellCheck | Not installed by default | `brew install shellcheck` |
| Hadolint | Not installed by default | `brew install hadolint` |
| GHA Docker layer cache (`type=gha`) | Not available locally — local builds use the Docker daemon's own layer cache | No workaround; local cache still avoids full rebuilds |
| GitHub artifact upload | `actions/upload-artifact` does not run locally | Inspect files directly inside the container after the test run |

After installing shellcheck and hadolint, `make ci-local` has full Tier 1 parity
with CI.

---

## Quick reference

```bash
# Pre-push: run Tier 1 + docker-dev smoke tests (~10-15 min)
make ci-local

# Before merging a PR that touches hw/ or patches/: full pipeline
make ci-full                    # ~40-50 min cold; cached after first builder build

# Tier 1 only (fastest feedback loop, ~2 min)
make check-versions
make lint
uv run pytest tests/repl2qemu/ tests/test_yaml2qemu.py \
               tests/test_cli_generator.py tests/test_fdt_emitter.py -v

# Docker stages only (when the Docker build is what's red in CI)
make docker-dev                 # base → toolchain → devenv with smoke tests
make docker-builder             # the slow QEMU compile stage
make docker-runtime             # lean runtime image

# One specific phase smoke test inside the builder container
docker run --rm -v "$(pwd):/workspace" -w /workspace \
  -e PYTHONPATH=/workspace -e VIRTMCU_STALL_TIMEOUT_MS=60000 \
  virtmcu-builder:dev \
  bash -c "make -C test/phase1 && bash test/phaseN/smoke_test.sh"

# Sync versions after editing VERSIONS
make sync-versions && make check-versions
```
