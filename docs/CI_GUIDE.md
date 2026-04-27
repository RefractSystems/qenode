# CI/CD & Reliability Guide

How to understand the optimized CI pipeline, reproduce failures locally, and understand every mount and cache involved.

---

## 1. Local CI Gates — Three Levels

There is no magic here. Every level is documented so that agents and humans can reason about what runs where, on which filesystem, and why.

### Level 0 — Git Hooks (fast, no Docker)

Installed by `make install-hooks`. Both `pre-commit` and `pre-push` run:
```bash
make lint && make test-unit
```
**Directly in the current shell** — no `docker run` involved.

Why no Docker: the devcontainer is built from the same `devenv-base` image that GitHub CI uses. Running directly in it gives identical toolchain coverage without the CARGO_HOME conflict that corrupted Cargo's fingerprint cache in earlier designs (see §6 for the full story).

Bypass when needed: `git commit --no-verify` / `git push --no-verify`.

### Level 1 — `make ci-local` (GitHub Tier 1 simulation, ~5-10 min)

Builds the `devenv-base` image locally (or uses a cached layer), then runs the **exact same three `docker run` commands** that `.github/workflows/ci-main.yml` executes:

```
make lint          (ruff, clippy -D warnings, check-ffi, hadolint, …)
make build-tools   (virtmcu-tools Python wheel validation)
make test-unit     (Rust unit tests + Python unit tests, no QEMU)
```

Run this before opening a pull request.

### Level 2 — `make ci-full` (authoritative parity, ~15-20 min warm / ~40 min cold)

`ci-local` + `ci-asan` + `ci-miri` + full `builder` Docker image build (QEMU compiled inside) + every smoke phase run sequentially inside that image via `scripts/ci-phase.sh all`.

This is the authoritative "will GitHub be green?" answer. Run this before merging to main.

---

## 2. Workflow Split: `ci-pr.yml` vs `ci-main.yml`

### Why two workflows instead of one

The original `ci.yml` handled both pull requests and pushes to `main` in a single file. This was correct but had two problems:

1. **Implicit coupling.** Publish jobs (devenv, runtime images) were guarded by `if: github.event_name == 'push'` conditionals scattered through the file. A reviewer had to trace each job's `if:` chain to understand what runs when. One misplaced condition would silently publish on a PR.

2. **Wasted runners on PRs.** On every PR, GitHub still evaluated publish job conditions and spun up job runners before they immediately exited. Minor, but unnecessary noise in the GitHub Actions UI.

The split makes each file's purpose unambiguous:

| File | Trigger | Purpose |
|---|---|---|
| `ci-pr.yml` | `pull_request` + `merge_group` | Validate the change. Never publish. |
| `ci-main.yml` | `push: main` + `tags: v*.*.*` | Validate + publish images to GHCR. |

### `merge_group:` trigger

`ci-pr.yml` listens on `merge_group:` in addition to `pull_request`. This enables GitHub's [Merge Queue](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/configuring-pull-request-merges/managing-a-merge-queue): PRs are validated against the tip of main (plus any queued PRs ahead of them) before the merge is committed. The common failure mode — "PR was green but broke main because another PR landed first" — is eliminated. Used by rust-lang/rust, Firefox, and most high-throughput teams.

### Architecture matrix

PRs validate amd64 only. arm64 takes ~40 min and is redundant for change validation — the QEMU core and Rust plugin ABI are architecture-independent at the source level. Main and tags build both arches because they produce the multi-arch manifests that users actually pull.

| Event | amd64 | arm64 |
|---|---|---|
| `pull_request` / `merge_group` | ✓ | — |
| `push: main` / `tag` | ✓ | ✓ |

---

## 3. Mount Strategy — What Lives Where

Every `docker run devenv-base` call in the Makefile uses these mounts. There is no `.cargo-cache` host-path bind-mount anymore.

| Mount | Host side | Container path | Type | Purpose |
|---|---|---|---|---|
| Workspace | `$(CURDIR)` | `/workspace` | bind | Source code, test scripts, generated `.venv-docker`, `test-results/` |
| Cargo registry | Docker named volume `ci-cargo-registry` | `/usr/local/cargo/registry` | named volume | Downloaded crate source tarballs. Persists across runs; never touches the host filesystem |
| Compiled artifacts | (none — ephemeral) | `/tmp/ci-target` via `CARGO_TARGET_DIR` | tmpfs inside container | Rust `.rlib`/binary outputs. Disappears when the container exits |
| Python venv | `$(CURDIR)/.venv-docker` (inside workspace bind) | `/workspace/.venv-docker` via `UV_PROJECT_ENVIRONMENT` | bind (subdir) | Isolated Python environment so host `.venv` and container venv never mix |

**What is explicitly NOT mounted into the container:**
- `$(CURDIR)/target/` — the host's Cargo target directory. This is the critical exclusion. Sharing it between the host (CARGO_HOME=/usr/local/cargo) and the container would cause fingerprint corruption (see §6).
- `$(CURDIR)/.cargo-cache/` — the old host-path registry cache. Replaced by the `ci-cargo-registry` named volume.

**The builder-image smoke tests** (in `ci-full` and `test-integration-docker`) use the same bind-mount for `/workspace` but target the `builder` image instead of `devenv-base`. The `builder` image already has QEMU, all `.so` plugins, and the test tools (`zenoh_coordinator`, `mujoco_bridge`, `resd_replay`) baked into `/opt/virtmcu/`. No source compilation happens during smoke tests.

### Named Docker Volume Lifecycle

```bash
# Inspect
docker volume inspect ci-cargo-registry

# Prune if corrupted or you need a clean slate
docker volume rm ci-cargo-registry

# ci-local will recreate it automatically on next run
```

---

## 4. Docker Image Hierarchy

```
rust-builder          (Stage 0)  Rust toolchain + bindgen + cargo-audit etc.
base                  (Stage 1)  Debian slim + vscode user + uv + gh CLI
toolchain             (Stage 2)  base + build deps + ARM toolchain + Python + CMake + FlatBuffers
flatcc-builder        (Stage 3)  Temporary: builds flatcc from source
simulation-toolchain  (Stage 4)  toolchain + flatcc + zenoh-c + Rust (from rust-builder)
qemu-builder          (Stage 5)  simulation-toolchain + QEMU cloned + base QEMU compiled
builder               (Stage 6)  qemu-builder + hw/ plugins + tools binaries → installed to /opt/virtmcu
devenv-base           (Stage 7)  toolchain + Rust + Node.js + Python deps (NO QEMU baked in)
devenv                (Stage 8)  devenv-base + /opt/virtmcu copied from builder
runtime               (Stage 9)  base + Python deps + /opt/virtmcu (lean runtime image)
```

**devenv-base vs devenv:**
- `devenv-base` is what GitHub Tier 1 and `make ci-local` use — it has the full toolchain but no QEMU binary. Fast to build, used for lint + unit tests.
- `devenv` is the developer-facing image used in `.devcontainer/`. It includes the pre-built QEMU from `builder`.

**Why test tools are built in Stage 6 (`builder`), not in a separate stage:**
A previous design had a `test-tools-builder` stage that copied a subset of the workspace and used `sed` to prune `Cargo.toml` members. This was fragile: new workspace members broke it silently, and the wrong binary name (`cyber_bridge` package produces `mujoco_bridge` + `resd_replay`, not a binary called `cyber_bridge`). Stage 6 already has the full workspace under `/build/virtmcu/{hw,tools}`, so all members are present and no pruning is needed.

**Why QEMU is compiled with `-j$(nproc)`:**
Previously the Dockerfile capped parallel jobs to 1 in CI (`if [ "$CI" = "true" ]; then JOBS=1`). This was written for old GitHub runners (2 CPU, 7 GB RAM). Current `ubuntu-latest` runners have 4 CPUs and 16 GB RAM; `ubuntu-24.04-arm` is the same. QEMU's C compilation has no unusual per-process memory pressure at these job counts. Removing the cap cuts the cold QEMU build from ~40 min to ~12-15 min. The `CI` build arg was removed from the Dockerfile entirely because it had no other use.

---

## 5. Pipeline Overview (GitHub CI Tiers)

### PR workflow (`ci-pr.yml`)

```
changes ──┬──────────────────────────────────────────────────────┐
          │ (skip if only docs/md changed)                       │
          ▼                                                       ▼
     tier1-checks          generate-matrix ────────────► build-qemu (amd64)
     (lint + unit)     (reads smoke-phases.json)               │
          │                     │                   ┌──────────┴──────────┐
          └─────────────────────┴──────────────────►▼                     ▼
                                              smoke-tests          firmware-coverage
                                              (N phases, amd64)
                                                    │
                                                    ▼
                                            peripheral-coverage
```

No publish jobs. The builder image is pushed to GHCR tagged `sha-<sha>-amd64` so smoke-test runners can pull it without rebuilding. The SHA tag is ephemeral — it is never promoted to `latest`.

### Main/tag workflow (`ci-main.yml`)

```
changes ──┬──────────────────────────────────────────────────────────────────┐
          │ (skip if only docs/md changed, unless it's a tag)               │
          ▼                                                                   ▼
     tier1-checks    setup ──► build-qemu (amd64 + arm64 in parallel)
     (lint + unit)                         │
          │          generate-matrix        │
          │      (reads smoke-phases.json)  │
          ▼                │               │
  publish-devenv-base ─────┘               │
  (amd64 + arm64)          │               │
          │                └──────────────►▼
          ▼                       smoke-tests (N phases × 2 arches)
   publish-devenv                          │              ┌────────────┐
   (amd64 + arm64)                         │              │ firmware-  │
          │                                ├─────────────►│ coverage   │
          ▼                                ▼              └─────┬──────┘
   merge-devenv                    peripheral-coverage          │
   (multi-arch manifest)                   │                    │
                                           ▼                    │
                                   publish-runtime (amd64 + arm64) <──┘
                                           │
                                           ▼
                                   merge-runtime (multi-arch manifest)
```

### Tier 1 — Static Analysis & Unit Tests
GitHub runs three `docker run devenv-base` commands sequentially with no `CARGO_HOME` override. `make ci-local` mirrors this exactly using the same image, same flags, and same command sequence.

### Tier 2 — Build QEMU
`docker buildx bake builder` compiles QEMU and all Rust plugins inside the multi-stage Dockerfile. The resulting image is pushed to GHCR and tagged with the commit SHA so Tier 3 runners can pull it without rebuilding. On PRs only amd64 is built. On main/tags both arches are built in parallel on native runners (`ubuntu-latest` for amd64, `ubuntu-24.04-arm` for arm64).

### Tier 3 — Smoke Tests
Each phase is an independent `docker run builder bash scripts/ci-phase.sh <phase>` job. The phases run in parallel across up to 40 GitHub runners (20 phases × 2 arches on main; 20 phases × 1 arch on PRs). `scripts/ci-phase.sh` is the single source of truth for what each phase does — it is used by both GitHub and `make ci-full`.

### Tier 4 — Coverage
Collects `.gcda` files produced during Tier 3 (via `GCOV_PREFIX`) and runs `gcovr` inside the builder image to produce unified reports.

### Tier 5 — Publish (main/tags only)
Pushes `devenv` and `runtime` multi-arch manifests to GHCR after all smoke tests and coverage pass. The manifest step (`merge-devenv`, `merge-runtime`) combines the per-arch images into a single multi-arch tag using `docker buildx imagetools create`.

---

## 6. Cache Architecture

### The three-layer cache

The QEMU build (`qemu-builder` stage, Stage 5) is the most expensive step — 12-15 min warm, 15-40 min cold depending on runner speed. Three cache mechanisms work together in priority order:

```
1. Registry cache (primary, persistent)
   build-cache:builder-amd64   ← written by main AND PRs, mode=max
   build-cache:toolchain-amd64 ← etc.
   Warm hit: ~2-3 min for the full builder image

2. GHA cache (fallback, per-repo 10 GB limit, 7-day TTL)
   scope=virtmcu-builder-amd64  ← written when registry cache is not being written
   scope=virtmcu-toolchain-amd64
   etc. (one scope per stage, not shared)
   Warm hit: ~3-5 min pull from GHA cache

3. Cold build (no cache)
   Full QEMU compile from scratch: ~12-15 min with -j$(nproc)
```

### Why PRs write the registry cache

Previously only main wrote to `build-cache:builder-amd64`. A PR on a runner with a cold GHA cache had no registry to read from and always triggered a full 40-min QEMU compile.

PRs now set `PUSH_CACHE: "true"` for `build-qemu`. This writes the intermediate layer cache (`build-cache:builder-amd64,mode=max`) without touching image tags — the `latest` and semver tags are applied only by `docker-bake-latest.hcl` and `docker-bake-release.hcl`, which are excluded from PR bake invocations. Safe: the build cache and the published image tags are separate namespaces in GHCR.

### Docker Bake Tag Replacement Rule

**IMPORTANT TECHNICAL GOTCHA:** In Docker Bake, when a target is redefined across multiple files (e.g., `docker-bake.hcl` and `docker-bake-release.hcl`), the `tags` array is **REPLACED**, not merged.

- **The Problem:** If `docker-bake.hcl` defines `tags = ["sha-<sha>"]` and `docker-bake-release.hcl` defines `tags = ["v1.2.3"]`, the resulting image will *only* have the `v1.2.3` tag.
- **The Impact:** Our manifest merge jobs depend on the `sha-<sha>` tags. If they are missing because a release tag overwrote them, the multi-arch manifest creation will fail.
- **The Fix:** `docker-bake-release.hcl` and `docker-bake-latest.hcl` must explicitly include both the semantic tag and the SHA-based tag.

### Why GHA cache uses per-stage scopes

The original design used a single scope `virtmcu-amd64` shared by all six targets. Without `mode=max`, each target's `cache-to` only writes its own final layer. The last target to write (builder) overwrites earlier entries, so base and toolchain layers were lost from the GHA cache. Each target now gets its own scope:

```
virtmcu-base-amd64
virtmcu-toolchain-amd64
virtmcu-devenv-base-amd64
virtmcu-builder-amd64
virtmcu-devenv-amd64
virtmcu-runtime-amd64
```

The GHA cache uses default mode (final layer only). `mode=max` (all intermediate layers) is reserved for the registry cache, which is not subject to the 10 GB per-repo GHA cap.

### What invalidates the QEMU layer cache

The `qemu-builder` stage (and everything above it) is invalidated when any of these change:

| Input | Invalidates |
|---|---|
| `QEMU_VERSION` in `BUILD_DEPS` | Everything from qemu-builder up |
| `ZENOH_VERSION` in `BUILD_DEPS` | Everything from simulation-toolchain up |
| `patches/` directory | qemu-builder and builder |
| `scripts/apply-qemu-patches.sh` | qemu-builder and builder |
| `docker/Dockerfile` (configure flags) | qemu-builder and builder |
| `hw/` Rust sources | builder only (QEMU core cache survives) |
| `tools/` Rust sources | builder only (QEMU core cache survives) |
| `Cargo.toml` / `Cargo.lock` | builder only |

A PR that changes only `hw/` triggers a `qemu-builder` cache hit and only rebuilds the Rust plugin layer — typically 3-5 min.

### Known cache race: concurrent PRs

Two PRs running `build-qemu` simultaneously both write to `build-cache:builder-amd64`. The last writer wins for the manifest pointer. This is a cache race, not a correctness race: the worst outcome is a cache miss on the losing PR's next push, not an incorrect test result. BuildKit uses content-addressed storage, so if both PRs built from the same QEMU/patches inputs, they produce identical layers anyway.

---

## 7. The CARGO_HOME Corruption Story (Why the Design Is What It Is)

This section exists so agents and humans never repeat the same mistake.

**The old design** had three Cargo environments sharing the same `target/` directory:

| Context | CARGO_HOME | target/ |
|---|---|---|
| Direct `cargo` in devcontainer | `/usr/local/cargo` (baked) | `/workspace/target` |
| `make ci-local` docker run | `/workspace/.cargo-cache` (overridden) | `/workspace/target` (bind-mounted) |
| GitHub CI docker run | no override → `/usr/local/cargo` | `/workspace/target` |

Cargo embeds the registry source path in every `.rlib` fingerprint. When `make ci-local` overrode `CARGO_HOME` to `/workspace/.cargo-cache` but mounted the same `/workspace/target` that the devcontainer had compiled against `/usr/local/cargo`, Cargo saw the existing `.rlib` files as belonging to a different registry and tried to recompile. Partial downloads (from interrupted runs) left the registry in an inconsistent state. Result: "can't find crate for `proc_macro2`" errors that disappeared after `rm -rf /workspace/.cargo-cache/registry` — until the next interrupted run.

**The fix has three parts:**
1. `CARGO_TARGET_DIR=/tmp/ci-target` — for `make ci-local` and docker-in-docker steps, compiled artifacts stay inside the container and vanish on exit.
2. Named volume `ci-cargo-registry` at `/usr/local/cargo/registry` — crate downloads persist across `docker run` invocations without a host-path bind.
3. **Devcontainer Volumes** — The `.devcontainer.json` mounts named volumes over `/workspace/target` and `/usr/local/cargo/registry`. This prevents the host machine's `rust-analyzer` from concurrently compiling and corrupting the devcontainer's registry cache via the hypervisor bind mount.

**Rule:** Never add `-e CARGO_HOME=<host-path>` to a `docker run` that also mounts the workspace. If you need to cache downloads across runs, use a named Docker volume.

---

## 8. Reliability Architecture (The "Safe Workspace")

### Workspace-Scoped Cleanup (`cleanup-sim.sh`)
Inspects `/proc/<pid>/cwd` and `/proc/<pid>/cmdline`. Only kills orphaned QEMU/Zenoh processes that originated from the active workspace directory — other agents' simulations in other directories are untouched.

### The FFI Gate (`check-ffi.py`)
Uses `pahole` to extract struct offsets directly from the compiled `qemu-system-arm` binary and compares them against the `assert!` statements in Rust. Layout drift → loud failure before any simulation starts.

### Dynamic Port Allocation (`get-free-port.py`)
Tests never use hardcoded ports. Every parallel worker gets its own ephemeral Zenoh router via the `zenoh_router` pytest fixture or `scripts/get-free-port.py` in bash.

---

## 9. Test Script Conventions

| Assumption | Safe / Recommended | Banned |
| :--- | :--- | :--- |
| Ports | `scripts/get-free-port.py`, `zenoh_router` fixture | Hardcoded numbers (7447, 1234) |
| File generation | `tmp_path` fixture, `mktemp -d` | Writing to `workspace_root` or `test/phaseX/` |
| Uniqueness | `os.getpid()`, `worker_id` | `random.randint()` |
| Daemons | Reusable `pytest` fixtures | `subprocess.Popen("cargo run...")` |
| Temp dirs | `tempfile.mkdtemp()` or `/tmp/virtmcu-test-*` | Fixed `/tmp/my_test_data` |
| Process kill | Trust workspace scoping | `pkill qemu` (global kill) |
| QEMU path | `scripts/run.sh` | `/opt/virtmcu/bin/...` (absolute) |
| Cleanup | Fixture / Makefile | `make clean-sim` inside a fixture |

---

## 10. Reproducing a GitHub CI Failure Locally

1. `make check-ffi` — verify FFI layout is valid.
2. `make ci-local` — reproduce Tier 1 failures (lint/build-tools/unit-tests).
3. For a specific smoke phase:
   ```bash
   # Build or pull the builder image first
   bash scripts/docker-build.sh builder
   # Run the failing phase
   docker run --rm \
     -v "$(pwd):/workspace" -w /workspace \
     -e PYTHONPATH=/workspace \
     -e VIRTMCU_STALL_TIMEOUT_MS=120000 \
     -e USER=vscode \
     ghcr.io/refractsystems/virtmcu/builder:dev-amd64 \
     bash scripts/ci-phase.sh <PHASE_NUMBER>
   ```
4. For the full matrix: `make ci-full` (runs phases sequentially, same script as GitHub).

---

## 11. Troubleshooting

| Symptom | Cause | Fix |
| :--- | :--- | :--- |
| `can't find crate for proc_macro2` / `rand` / etc. | Cargo fingerprint corruption (CARGO_HOME mismatch with shared target/) | `docker volume rm ci-cargo-registry` then re-run. Never use `-e CARGO_HOME=<host-path>` with a mounted workspace. |
| `SIGSEGV` in plugin | FFI layout drift | `scripts/check-ffi.py --fix` |
| `Address already in use` | Hardcoded port | Switch to `scripts/get-free-port.py` |
| `Permission denied` in /workspace | UID mismatch in container | `sudo chown -R 1000:1000 .` |
| CI STALL | Runner load spike or ASan overhead | The system automatically scales `VIRTMCU_STALL_TIMEOUT_MS` to 300s under ASan. If stalls persist, check logs for deadlocks or manually raise the env var. |
| `cyber_bridge` binary not found | Wrong binary name | The `cyber_bridge` package produces `mujoco_bridge` and `resd_replay` — never a binary called `cyber_bridge`. |
| GHA cache miss after a repo rename or workflow rename | GHA cache scopes are keyed to the repo | Nothing to do — the registry cache (`build-cache:*`) is the primary. GHA is the fallback and will repopulate over a few PR runs. |

---

## 12. Clock Sync & Stall Timeouts

To maintain deterministic co-simulation, `zenoh-clock` enforces a **Stall Timeout**. If QEMU fails to reach a virtual-time boundary within a certain wall-clock window, it reports a STALL (error code 1).

### Enterprise Policy: Environment-Driven Timeouts
We avoid hardcoding `stall-timeout` in test files. Instead, the system uses a centralized scaling policy:

1. **Default**: 5 seconds (`VIRTMCU_STALL_TIMEOUT_MS=5000`).
2. **ASan/UBSan**: 300 seconds (5 minutes). This is automatically injected by `tests/conftest.py` when `VIRTMCU_USE_ASAN=1` is detected.
3. **Manual Override**: You can override this globally by setting the `VIRTMCU_STALL_TIMEOUT_MS` environment variable.

### Why we don't hardcode in tests
- **Portability**: Different runners have different performance profiles.
- **Fail-Fast**: Standard runs should fail quickly if a deadlock occurs.
- **Maintenance**: Changing the global timeout policy happens in one place (`conftest.py`), not in 50 test files.

If you encounter a `RuntimeError: Node X reported CLOCK STALL`, it means QEMU stopped making progress. Check the QEMU logs for BQL deadlocks or extremely slow TCG translation.

---

## 13. Known Limitations and Future Improvements

These are real gaps in the current design, documented here so they are not forgotten and not re-invented from scratch.

### 13.1 Workflow duplication (`workflow_call`)

`ci-pr.yml` and `ci-main.yml` share ~300 lines of identical step definitions (lint, build-qemu, smoke-tests, coverage jobs). They differ only in triggers, the arch matrix, and whether publish jobs are present.

**Smoke phase duplication is solved**: both workflows now read `.github/smoke-phases.json` via a `generate-matrix` job, so adding or removing a smoke phase requires editing one file only (see §13.6).

The remaining duplication (step-level YAML for lint, build-qemu, coverage) is addressed by [reusable workflows](https://docs.github.com/en/actions/sharing-automations/reusing-workflows): a `_ci-shared.yml` with `on: workflow_call:` inputs for `arches` and `push_cache`, called by both files, would eliminate the rest. Not done yet because it requires restructuring job `needs:` graphs across file boundaries, which needs careful testing against the merge_group trigger specifically.

### 13.2 ccache persistence across CI runs

The Dockerfile uses `--mount=type=cache,target=/var/cache/ccache` for QEMU's C compilation. This BuildKit cache mount is ephemeral: it persists within a single `docker build` invocation but vanishes between separate invocations on different runners.

For true ccache persistence across CI runs, the ccache directory would need to be stored in a GHA cache and bind-mounted into the Buildx build context. This is not straightforward with `docker/bake-action` — Buildx does not support arbitrary host-path mounts for build cache. The recommended path is `USE_CCACHE=true` combined with a remote ccache server (e.g., sccache with S3 backend), but this adds infrastructure complexity.

Currently the registry cache (`build-cache:builder-amd64,mode=max`) covers the same use case more reliably for the common scenario (QEMU unchanged between PRs). ccache only adds value when the Docker layer cache is cold and only some QEMU C files changed — a narrow scenario.

### 13.3 QEMU-specific change detection for skipping the full build

Currently, `build-qemu` runs whenever any source file changes (the `changes` filter excludes only docs and markdown). A PR that changes only `hw/` Rust code still triggers the full bake pipeline — although the `qemu-builder` Docker layer is a cache hit, pulling it from GHCR takes 2-3 min.

A more aggressive optimization: detect whether QEMU-related inputs changed (`patches/`, `scripts/apply-qemu-patches.sh`, QEMU/Zenoh version lines in `BUILD_DEPS`, Dockerfile configure flags). If nothing QEMU-related changed, retag the last main `builder:latest-amd64` as `builder:sha-<sha>-amd64` and skip the bake entirely. This would reduce the common "hw/ change only" PR from ~5 min to ~30 seconds.

Requires a new `qemu_changed` output from the `changes` job, a conditional retag step, and careful handling of the `push: true` step that smoke tests depend on.

### 13.4 Fork PR security (if the repo becomes public)

PR builds now write to `build-cache:builder-amd64` in GHCR (`packages: write`). For a private repo with internal contributors this is safe. If the repo is ever made public, fork PRs from external contributors should not be able to write to the build cache — a malicious fork could poison it with a modified QEMU or backdoored plugin.

The fix: split `build-qemu` into two jobs — `build-qemu-pr` (no cache write, for fork PRs) and `build-qemu-internal` (cache write, only for same-repo PRs, gated on `github.event.pull_request.head.repo.full_name == github.repository`). GitHub's default fork PR token already restricts `packages: write`, so this may be a no-op in practice, but the explicit job split makes the security boundary visible.

### 13.5 GHA cache cold start after scope rename

The GHA cache scopes were renamed from `virtmcu-amd64` (shared across all stages) to per-stage names (`virtmcu-builder-amd64`, `virtmcu-toolchain-amd64`, etc.). All existing GHA cache entries under the old scope names were orphaned on deploy. The registry cache (`build-cache:*`) is the primary and repopulates GHA naturally over a few PR runs, so there is no action required — but the first few PRs after the rename will take a full registry pull instead of a GHA hit.

### 13.6 Dynamic smoke matrix (`smoke-phases.json`) — Design and Risks

#### What it does

Both workflows generate the `smoke-tests` matrix at runtime via a `generate-matrix` job that reads `.github/smoke-phases.json` using `jq`. This replaces 20-entry hardcoded `include:` blocks in each workflow file, making `.github/smoke-phases.json` the **single source of truth** for phase names and ordering.

To add or remove a smoke phase: edit `.github/smoke-phases.json` only. Both `ci-pr.yml` and `ci-main.yml` pick up the change automatically on the next run.

#### Known risks and mitigations

**Risk: Branch protection check names are coupled to `name` fields in the JSON.**
GitHub branch protection "required status checks" are matched by job name string. A job named `Smoke — Phase 7 (Zenoh Clock) (amd64)` in branch protection will break if the `name` field in `smoke-phases.json` is edited — even a cosmetic rename silently removes that check from the protection list, making the branch unprotectable.
- **Mitigation:** Treat `phase` and `name` fields in `smoke-phases.json` as stable identifiers. Rename only after updating branch protection rules. The `phase` field (used in artifact names and `ci-phase.sh` routing) is the stable key; the `name` field is display-only and change-sensitive for branch protection.

**Risk: `generate-matrix` always runs, even when `changes.expensive == 'false'`.**
The job does only a checkout + a `jq` call (~10 seconds), so this is negligible. `smoke-tests` still carries `if: needs.changes.outputs.expensive == 'true'` and is skipped when changes don't warrant it. The unconditional `generate-matrix` is intentional: it ensures the matrix output is always available as a job dependency, avoiding the "output from skipped job is empty" failure mode.

**Risk: actionlint cannot statically verify a dynamic matrix.**
`actionlint` sees `matrix: ${{ fromJson(...) }}` and cannot enumerate valid matrix keys at lint time. It emits warnings for `matrix.name`, `matrix.arch`, `matrix.os`, `matrix.phase` in the job body since they don't appear in a static `matrix:` block. The CI lint step currently does not run `actionlint`; if added in the future, suppress those specific dynamic-matrix warnings with `# actionlint:ignore` comments on the affected lines.

**Risk: `actuator` phase is intentionally absent from the matrix.**
`ci-phase.sh all` runs the actuator phase locally, but `smoke-phases.json` deliberately excludes it from the CI matrix (it has infrastructure prerequisites not available on GitHub runners). Any contributor adding `actuator` to `smoke-phases.json` must also ensure the builder image includes those prerequisites.
