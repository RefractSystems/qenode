# Reproducible Build Architecture

VirtMCU ensures "It works on my machine" and "It works in CI" are the same statement by utilizing a containerized, content-addressed build architecture. This chapter explains the design of our build pipeline and how to utilize it for local development.

## 1. The Container Hierarchy

Our environment is built using a tiered Docker strategy to maximize caching and minimize setup time:

1.  **Base Layer (`base`)**: Contains the OS (Debian), core build tools (GCC, LLVM, Python), and foundational C libraries.
2.  **Toolchain Layer (`toolchain`)**: Adds the pinned Rust nightly toolchain, `mdbook`, and specialized binary tools like `hadolint` and `actionlint`.
3.  **Third-Party Builder (`third-party-base`)**: A specialized, ephemeral stage that clones QEMU, applies our patches (see [RFC-0030](../rfcs/0030-qemu-patch-strategy.md)), and builds the core emulator. This stage is cached heavily in GitHub Actions to avoid 20-minute rebuilds.
4.  **Developer Environment (`devenv`)**: The final image used by VS Code Dev Containers. It layers the project source code and pre-compiled third-party artifacts onto the toolchain.

## 2. Docker Bake (Multi-Stage Orchestration)

We use `docker buildx bake` (configured via `docker-bake.hcl`) to manage these stages. Unlike a standard `docker build`, Bake allows us to:
- Build multiple architectures (amd64, arm64) in parallel.
- Define complex dependencies between stages (e.g., the `ci` image depends on the `third-party-base` output).
- Utilize content-addressed tagging (`THIRD_PARTY_CACHE_TAG`) to ensure we only rebuild QEMU when the `patches/` or `BUILD_DEPS` change.

## 3. Local Usage

While the `Makefile` provides the primary interface, it delegates all heavy lifting to the containers:

- `make docker-dev`: Rebuilds the local dev container stages and runs smoke tests.
- `make sync-versions`: Propagates version pins from `BUILD_DEPS` into all project configuration files.
- `make bootstrap`: The "first-run" command that initializes the container and builds QEMU for the first time.

## 4. The `virtmcu-cli` Orchestrator

The `virtmcu-cli` (written in Rust) is the glue between the build system and the simulation. It handles:
- **Patch Management**: Applying surgical fixes to the QEMU submodule.
- **Topology Generation**: Converting World YAMLs into QEMU command-line arguments.
- **Version Syncing**: Ensuring that `Cargo.toml`, `package.json`, and Dockerfiles all use the same version of Zenoh and QEMU.

## 5. CI Parity

The `ci` image is identical to the `devenv` image but optimized for non-interactive execution. When you run `make ci-check` locally, you are executing the exact same bits that will run in the GitHub Actions runner. This eliminates "Passes locally, fails in CI" syndrome.

---

### See Also
- [RFC-0030: QEMU Patch Strategy](../rfcs/0030-qemu-patch-strategy.md)
- [Testing Strategy](./03-testing-strategy.md)
