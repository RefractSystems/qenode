# docker-bake.hcl — Single Source of Truth for virtmcu Docker Builds


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
variable "VIRTMCU_IMAGE_REGISTRY" {}
variable "VIRTMCU_DEVENV_IMAGE" {}
variable "VIRTMCU_CI_IMAGE" {}

variable "THIRD_PARTY_CACHE_TAG" {
  default = "latest"
}

# ── Groups ────────────────────────────────────────────────────────────────────

group "default" {
  targets = ["base", "rust-builder", "toolchain", "devenv", "flatcc-builder"]
}

group "all" {
  targets = ["base", "rust-builder", "toolchain", "devenv", "flatcc-builder", "third-party-base", "third-party-base-asan", "ci", "ci-asan"]
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
  tags     = ["${VIRTMCU_IMAGE_REGISTRY}/base:${IMAGE_TAG}-${ARCH}"]
  cache-from = [
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:base-${ARCH}",
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/base:latest-${ARCH}",
    "type=gha,scope=virtmcu-base-${ARCH}"
  ]
  cache-to = CI == "true" ? (
    PUSH_CACHE == "true" ? [
      "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:base-${ARCH},mode=max"
    ] : [
      "type=gha,scope=virtmcu-base-${ARCH}"
    ]
  ) : []
}


target "rust-builder" {
  inherits = ["_common"]
  target   = "rust-builder"
  tags     = ["${VIRTMCU_IMAGE_REGISTRY}/rust-builder:${IMAGE_TAG}-${ARCH}"]
  cache-from = [
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:rust-builder-${ARCH}",
    "type=gha,scope=virtmcu-rust-builder-${ARCH}"
  ]
  cache-to = CI == "true" ? (
    PUSH_CACHE == "true" ? [
      "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:rust-builder-${ARCH},mode=max"
    ] : [
      "type=gha,scope=virtmcu-rust-builder-${ARCH},mode=max"
    ]
  ) : []
}

target "toolchain" {
  inherits = ["_common"]
  target   = "toolchain"
  tags     = ["${VIRTMCU_IMAGE_REGISTRY}/toolchain:${IMAGE_TAG}-${ARCH}"]
  cache-from = [
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:toolchain-${ARCH}",
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/toolchain:latest-${ARCH}",
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:base-${ARCH}",
    "type=gha,scope=virtmcu-toolchain-${ARCH}"
  ]
  cache-to = CI == "true" ? (
    PUSH_CACHE == "true" ? [
      "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:toolchain-${ARCH},mode=max"
    ] : [
      "type=gha,scope=virtmcu-toolchain-${ARCH}"
    ]
  ) : []
}


target "devenv" {
  inherits = ["_common"]
  target   = "devenv"
  tags     = ["${VIRTMCU_IMAGE_REGISTRY}/${VIRTMCU_DEVENV_IMAGE}:${IMAGE_TAG}-${ARCH}"]
  cache-from = [
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:${VIRTMCU_DEVENV_IMAGE}-${ARCH}",
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/${VIRTMCU_DEVENV_IMAGE}:latest-${ARCH}",
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:rust-builder-${ARCH}",
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:toolchain-${ARCH}",
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:base-${ARCH}",
    "type=gha,scope=${VIRTMCU_DEVENV_IMAGE}-${ARCH}"
  ]
  cache-to = CI == "true" ? (
    PUSH_CACHE == "true" ? [
      "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:${VIRTMCU_DEVENV_IMAGE}-${ARCH},mode=max"
    ] : [
      "type=gha,scope=${VIRTMCU_DEVENV_IMAGE}-${ARCH}"
    ]
  ) : []
}

# ── third-party-base: frozen QEMU core, keyed by QEMU version + patches hash ────────
# Written only by ci-main.yml (never by ci-pr.yml, which excludes this target).
# ci-pr.yml reads it via the ci target's cache-from list below.
target "flatcc-builder" {
  inherits = ["_common"]
  target   = "flatcc-builder"
  tags     = ["${VIRTMCU_IMAGE_REGISTRY}/flatcc-builder:${IMAGE_TAG}-${ARCH}"]
  cache-from = [
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:flatcc-builder-${ARCH}",
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:${VIRTMCU_DEVENV_IMAGE}-${ARCH}",
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/${VIRTMCU_DEVENV_IMAGE}:latest-${ARCH}",
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:rust-builder-${ARCH}",
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:toolchain-${ARCH}",
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:base-${ARCH}",
    "type=gha,scope=virtmcu-flatcc-builder-${ARCH}"
  ]
  cache-to = CI == "true" ? (
    PUSH_CACHE == "true" ? [
      "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:flatcc-builder-${ARCH},mode=max"
    ] : [
      "type=gha,scope=virtmcu-flatcc-builder-${ARCH},mode=max"
    ]
  ) : []
}

target "third-party-base" {
  inherits = ["_common"]
  target   = "third-party-builder"
  tags     = ["${VIRTMCU_IMAGE_REGISTRY}/third-party-base:${THIRD_PARTY_CACHE_TAG}-${ARCH}"]
  cache-from = [
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/third-party-base:${THIRD_PARTY_CACHE_TAG}-${ARCH}",
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:${VIRTMCU_DEVENV_IMAGE}-${ARCH}",
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/${VIRTMCU_DEVENV_IMAGE}:latest-${ARCH}",
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:flatcc-builder-${ARCH}",
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:rust-builder-${ARCH}",
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:toolchain-${ARCH}",
    "type=gha,scope=virtmcu-third-party-base-${ARCH}"
  ]
  cache-to = CI == "true" ? (
    PUSH_CACHE == "true" ? [
      "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/third-party-base:${THIRD_PARTY_CACHE_TAG}-${ARCH},mode=max"
    ] : [
      "type=gha,scope=virtmcu-third-party-base-${ARCH}"
    ]
  ) : []
}

target "ci" {
  inherits = ["_common"]
  target   = "ci"
  tags     = ["${VIRTMCU_IMAGE_REGISTRY}/${VIRTMCU_CI_IMAGE}:${IMAGE_TAG}-${ARCH}"]
  cache-from = [
    # Frozen QEMU core — a cache hit here skips the 40-minute QEMU compile.
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/third-party-base:${THIRD_PARTY_CACHE_TAG}-${ARCH}",
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:${VIRTMCU_CI_IMAGE}-${ARCH}",
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:${VIRTMCU_DEVENV_IMAGE}-${ARCH}",
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/${VIRTMCU_DEVENV_IMAGE}:latest-${ARCH}",
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:flatcc-builder-${ARCH}",
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:rust-builder-${ARCH}",
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:toolchain-${ARCH}",
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:base-${ARCH}",
    "type=gha,scope=${VIRTMCU_CI_IMAGE}-${ARCH}"
  ]
  cache-to = CI == "true" ? (
    PUSH_CACHE == "true" ? [
      "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:${VIRTMCU_CI_IMAGE}-${ARCH},mode=max"
    ] : [
      "type=gha,scope=${VIRTMCU_CI_IMAGE}-${ARCH}"
    ]
  ) : []
}


target "third-party-base-asan" {
  inherits = ["_common"]
  target   = "third-party-builder"
  args = {
    VIRTMCU_USE_ASAN = "1"
  }
  tags     = ["${VIRTMCU_IMAGE_REGISTRY}/third-party-base:${THIRD_PARTY_CACHE_TAG}-asan-${ARCH}"]
  cache-from = [
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/third-party-base:${THIRD_PARTY_CACHE_TAG}-asan-${ARCH}",
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:${VIRTMCU_DEVENV_IMAGE}-${ARCH}",
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/${VIRTMCU_DEVENV_IMAGE}:latest-${ARCH}",
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:flatcc-builder-${ARCH}",
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:rust-builder-${ARCH}",
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:toolchain-${ARCH}",
    "type=gha,scope=virtmcu-third-party-base-asan-${ARCH}"
  ]
  cache-to = CI == "true" ? (
    PUSH_CACHE == "true" ? [
      "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:third-party-base-asan-${ARCH},mode=max"
    ] : [
      "type=gha,scope=virtmcu-third-party-base-asan-${ARCH}"
    ]
  ) : []
}

target "ci-asan" {
  inherits = ["_common"]
  target   = "ci"
  args = {
    VIRTMCU_USE_ASAN = "1"
  }
  tags     = ["${VIRTMCU_IMAGE_REGISTRY}/${VIRTMCU_CI_IMAGE}:${IMAGE_TAG}-asan-${ARCH}"]
  cache-from = [
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/third-party-base:${THIRD_PARTY_CACHE_TAG}-asan-${ARCH}",
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:${VIRTMCU_CI_IMAGE}-asan-${ARCH}",
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:${VIRTMCU_DEVENV_IMAGE}-${ARCH}",
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/${VIRTMCU_DEVENV_IMAGE}:latest-${ARCH}",
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:flatcc-builder-${ARCH}",
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:rust-builder-${ARCH}",
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:toolchain-${ARCH}",
    "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:base-${ARCH}",
    "type=gha,scope=${VIRTMCU_CI_IMAGE}-asan-${ARCH}"
  ]
  cache-to = CI == "true" ? (
    PUSH_CACHE == "true" ? [
      "type=registry,ref=${VIRTMCU_IMAGE_REGISTRY}/build-cache:${VIRTMCU_CI_IMAGE}-asan-${ARCH},mode=max"
    ] : [
      "type=gha,scope=${VIRTMCU_CI_IMAGE}-asan-${ARCH}"
    ]
  ) : []
}
