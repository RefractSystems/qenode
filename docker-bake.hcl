# docker-bake.hcl — Single Source of Truth for virtmcu Docker Builds

variable "REGISTRY" {
  default = "ghcr.io"
}

variable "IMAGE_NAME_LOWER" {
  default = "refractsystems/virtmcu"
}

variable "IMAGE_TAG" {
  default = "dev"
}

# Versions from VERSIONS file (passed via environment)
variable "HADOLINT_VERSION" {}
variable "ACTIONLINT_VERSION" {}
variable "MDBOOK_VERSION" {}
variable "MDBOOK_MERMAID_VERSION" {}
variable "MDBOOK_PDF_VERSION" {}
variable "DEBIAN_CODENAME" {}
variable "NODE_VERSION" {}
variable "PYTHON_VERSION" {}
variable "ARM_TOOLCHAIN_VERSION" {}
variable "QEMU_VERSION" {}
variable "ZENOH_VERSION" {}
variable "CMAKE_VERSION" {}
variable "RUST_VERSION" {}
variable "FLATBUFFERS_VERSION" {}
variable "FLATCC_VERSION" {}
variable "UV_VERSION" {}
variable "CARGO_BINSTALL_VERSION" {}

# Architecture handling
variable "ARCH" {
  default = "amd64"
}

variable "CI" {
  default = "false"
}

variable "PUSH_CACHE" {
  default = "false"
}

variable "USE_CCACHE" {
  default = "true"
}

variable "VIRTMCU_USE_ASAN" {
  default = "0"
}

# Content-addressed QEMU cache tag: set by CI to "${QEMU_VERSION}-${patches_hash}".
# Defaults to "latest" for local builds where the exact tag does not matter.
variable "QEMU_CACHE_TAG" {
  default = "latest"
}

# ── Groups ────────────────────────────────────────────────────────────────────

group "default" {
  targets = ["base", "rust-builder", "toolchain", "devenv", "flatcc-builder"]
}

group "all" {
  targets = ["base", "rust-builder", "toolchain", "devenv", "flatcc-builder", "qemu-base", "qemu-base-asan", "ci", "ci-asan"]
}

# ── Common Configuration ──────────────────────────────────────────────────────

target "_common" {
  context = "."
  dockerfile = "docker/Dockerfile"
  secrets = [
    "GITHUB_TOKEN"
  ]
  args = {
    HADOLINT_VERSION      = HADOLINT_VERSION
    ACTIONLINT_VERSION    = ACTIONLINT_VERSION
    MDBOOK_VERSION        = MDBOOK_VERSION
    MDBOOK_MERMAID_VERSION = MDBOOK_MERMAID_VERSION
    MDBOOK_PDF_VERSION     = MDBOOK_PDF_VERSION
    DEBIAN_CODENAME       = DEBIAN_CODENAME
    NODE_VERSION          = NODE_VERSION
    PYTHON_VERSION        = PYTHON_VERSION
    ARM_TOOLCHAIN_VERSION = ARM_TOOLCHAIN_VERSION
    QEMU_REF              = "v${QEMU_VERSION}"
    ZENOH_C_REF           = ZENOH_VERSION
    CMAKE_VERSION         = CMAKE_VERSION
    RUST_VERSION          = RUST_VERSION
    FLATBUFFERS_VERSION   = FLATBUFFERS_VERSION
    FLATCC_VERSION        = FLATCC_VERSION
    UV_VERSION            = UV_VERSION
    CARGO_BINSTALL_VERSION= CARGO_BINSTALL_VERSION
    USE_CCACHE            = USE_CCACHE
    VIRTMCU_USE_ASAN      = "${VIRTMCU_USE_ASAN}"
  }
}

# ── Targets ───────────────────────────────────────────────────────────────────

target "base" {
  inherits = ["_common"]
  target   = "base"
  tags     = ["${REGISTRY}/${IMAGE_NAME_LOWER}/base:${IMAGE_TAG}-${ARCH}"]
  cache-from = [
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:base-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/base:latest-${ARCH}",
    "type=gha,scope=virtmcu-base-${ARCH}"
  ]
  cache-to = CI == "true" ? (
    PUSH_CACHE == "true" ? [
      "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:base-${ARCH},mode=max"
    ] : [
      "type=gha,scope=virtmcu-base-${ARCH}"
    ]
  ) : []
}


target "rust-builder" {
  inherits = ["_common"]
  target   = "rust-builder"
  tags     = ["${REGISTRY}/${IMAGE_NAME_LOWER}/rust-builder:${IMAGE_TAG}-${ARCH}"]
  cache-from = [
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:rust-builder-${ARCH}",
    "type=gha,scope=virtmcu-rust-builder-${ARCH}"
  ]
  cache-to = CI == "true" ? (
    PUSH_CACHE == "true" ? [
      "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:rust-builder-${ARCH},mode=max"
    ] : [
      "type=gha,scope=virtmcu-rust-builder-${ARCH},mode=max"
    ]
  ) : []
}

target "toolchain" {
  inherits = ["_common"]
  target   = "toolchain"
  tags     = ["${REGISTRY}/${IMAGE_NAME_LOWER}/toolchain:${IMAGE_TAG}-${ARCH}"]
  cache-from = [
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:toolchain-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/toolchain:latest-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:base-${ARCH}",
    "type=gha,scope=virtmcu-toolchain-${ARCH}"
  ]
  cache-to = CI == "true" ? (
    PUSH_CACHE == "true" ? [
      "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:toolchain-${ARCH},mode=max"
    ] : [
      "type=gha,scope=virtmcu-toolchain-${ARCH}"
    ]
  ) : []
}


target "devenv" {
  inherits = ["_common"]
  target   = "devenv"
  tags     = ["${REGISTRY}/${IMAGE_NAME_LOWER}/devenv:${IMAGE_TAG}-${ARCH}"]
  cache-from = [
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:devenv-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/devenv:latest-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:rust-builder-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:toolchain-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:base-${ARCH}",
    "type=gha,scope=virtmcu-devenv-${ARCH}"
  ]
  cache-to = CI == "true" ? (
    PUSH_CACHE == "true" ? [
      "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:devenv-${ARCH},mode=max"
    ] : [
      "type=gha,scope=virtmcu-devenv-${ARCH}"
    ]
  ) : []
}

# ── qemu-base: frozen QEMU core, keyed by QEMU version + patches hash ────────
# Written only by ci-main.yml (never by ci-pr.yml, which excludes this target).
# ci-pr.yml reads it via the ci target's cache-from list below.
target "flatcc-builder" {
  inherits = ["_common"]
  target   = "flatcc-builder"
  tags     = ["${REGISTRY}/${IMAGE_NAME_LOWER}/flatcc-builder:${IMAGE_TAG}-${ARCH}"]
  cache-from = [
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:flatcc-builder-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:devenv-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/devenv:latest-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:rust-builder-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:toolchain-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:base-${ARCH}",
    "type=gha,scope=virtmcu-flatcc-builder-${ARCH}"
  ]
  cache-to = CI == "true" ? (
    PUSH_CACHE == "true" ? [
      "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:flatcc-builder-${ARCH},mode=max"
    ] : [
      "type=gha,scope=virtmcu-flatcc-builder-${ARCH},mode=max"
    ]
  ) : []
}

target "qemu-base" {
  inherits = ["_common"]
  target   = "qemu-builder"
  tags     = ["${REGISTRY}/${IMAGE_NAME_LOWER}/qemu-base:${QEMU_CACHE_TAG}-${ARCH}"]
  cache-from = [
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/qemu-base:${QEMU_CACHE_TAG}-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:devenv-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/devenv:latest-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:flatcc-builder-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:rust-builder-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:toolchain-${ARCH}",
    "type=gha,scope=virtmcu-qemu-base-${ARCH}"
  ]
  cache-to = CI == "true" ? (
    PUSH_CACHE == "true" ? [
      "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/qemu-base:${QEMU_CACHE_TAG}-${ARCH},mode=max"
    ] : [
      "type=gha,scope=virtmcu-qemu-base-${ARCH}"
    ]
  ) : []
}

target "ci" {
  inherits = ["_common"]
  target   = "ci"
  tags     = ["${REGISTRY}/${IMAGE_NAME_LOWER}/ci:${IMAGE_TAG}-${ARCH}"]
  cache-from = [
    # Frozen QEMU core — a cache hit here skips the 40-minute QEMU compile.
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/qemu-base:${QEMU_CACHE_TAG}-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:ci-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:devenv-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/devenv:latest-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:flatcc-builder-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:rust-builder-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:toolchain-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:base-${ARCH}",
    "type=gha,scope=virtmcu-ci-${ARCH}"
  ]
  cache-to = CI == "true" ? (
    PUSH_CACHE == "true" ? [
      "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:ci-${ARCH},mode=max"
    ] : [
      "type=gha,scope=virtmcu-ci-${ARCH}"
    ]
  ) : []
}


target "qemu-base-asan" {
  inherits = ["_common"]
  target   = "qemu-builder"
  args = {
    VIRTMCU_USE_ASAN = "1"
  }
  tags     = ["${REGISTRY}/${IMAGE_NAME_LOWER}/qemu-base:${QEMU_CACHE_TAG}-asan-${ARCH}"]
  cache-from = [
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/qemu-base:${QEMU_CACHE_TAG}-asan-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:devenv-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/devenv:latest-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:flatcc-builder-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:rust-builder-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:toolchain-${ARCH}",
    "type=gha,scope=virtmcu-qemu-base-asan-${ARCH}"
  ]
  cache-to = CI == "true" ? (
    PUSH_CACHE == "true" ? [
      "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:qemu-base-asan-${ARCH},mode=max"
    ] : [
      "type=gha,scope=virtmcu-qemu-base-asan-${ARCH}"
    ]
  ) : []
}

target "ci-asan" {
  inherits = ["_common"]
  target   = "ci"
  args = {
    VIRTMCU_USE_ASAN = "1"
  }
  tags     = ["${REGISTRY}/${IMAGE_NAME_LOWER}/ci:${IMAGE_TAG}-asan-${ARCH}"]
  cache-from = [
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/qemu-base:${QEMU_CACHE_TAG}-asan-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:ci-asan-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:devenv-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/devenv:latest-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:flatcc-builder-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:rust-builder-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:toolchain-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:base-${ARCH}",
    "type=gha,scope=virtmcu-ci-asan-${ARCH}"
  ]
  cache-to = CI == "true" ? (
    PUSH_CACHE == "true" ? [
      "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:ci-asan-${ARCH},mode=max"
    ] : [
      "type=gha,scope=virtmcu-ci-asan-${ARCH}"
    ]
  ) : []
}
