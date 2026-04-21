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
  default = "false"
}

variable "VIRTMCU_USE_ASAN" {
  default = "0"
}

# ── Groups ────────────────────────────────────────────────────────────────────

group "default" {
  targets = ["base", "toolchain", "devenv-base"]
}

group "all" {
  targets = ["base", "toolchain", "devenv-base", "builder", "devenv", "runtime"]
}

# ── Common Configuration ──────────────────────────────────────────────────────

target "_common" {
  context = "."
  dockerfile = "docker/Dockerfile"
  args = {
    HADOLINT_VERSION      = HADOLINT_VERSION
    ACTIONLINT_VERSION    = ACTIONLINT_VERSION
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
    "type=gha,scope=virtmcu-${ARCH}"
  ]
  cache-to = CI == "true" ? (
    PUSH_CACHE == "true" ? [
      "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:base-${ARCH},mode=max"
    ] : [
      "type=gha,scope=virtmcu-${ARCH}"
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
    "type=gha,scope=virtmcu-${ARCH}"
  ]
  cache-to = CI == "true" ? (
    PUSH_CACHE == "true" ? [
      "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:toolchain-${ARCH},mode=max"
    ] : [
      "type=gha,scope=virtmcu-${ARCH}"
    ]
  ) : []
}

target "devenv-base" {
  inherits = ["_common"]
  target   = "devenv-base"
  tags     = ["${REGISTRY}/${IMAGE_NAME_LOWER}/devenv-base:${IMAGE_TAG}-${ARCH}"]
  cache-from = [
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:devenv-base-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/devenv-base:latest-${ARCH}",
    "type=gha,scope=virtmcu-${ARCH}"
  ]
  cache-to = CI == "true" ? (
    PUSH_CACHE == "true" ? [
      "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:devenv-base-${ARCH},mode=max"
    ] : [
      "type=gha,scope=virtmcu-${ARCH}"
    ]
  ) : []
}

target "builder" {
  inherits = ["_common"]
  target   = "builder"
  tags     = ["${REGISTRY}/${IMAGE_NAME_LOWER}/builder:${IMAGE_TAG}-${ARCH}"]
  cache-from = [
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:builder-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/builder:latest-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:toolchain-${ARCH}",
    "type=gha,scope=virtmcu-${ARCH}"
  ]
  cache-to = CI == "true" ? (
    PUSH_CACHE == "true" ? [
      "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:builder-${ARCH},mode=max"
    ] : [
      "type=gha,scope=virtmcu-${ARCH}"
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
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:devenv-base-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:builder-${ARCH}",
    "type=gha,scope=virtmcu-${ARCH}"
  ]
  cache-to = CI == "true" ? (
    PUSH_CACHE == "true" ? [
      "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:devenv-${ARCH},mode=max"
    ] : [
      "type=gha,scope=virtmcu-${ARCH}"
    ]
  ) : []
}

target "runtime" {
  inherits = ["_common"]
  target   = "runtime"
  tags     = ["${REGISTRY}/${IMAGE_NAME_LOWER}/runtime:${IMAGE_TAG}-${ARCH}"]
  cache-from = [
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:runtime-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/runtime:latest-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:base-${ARCH}",
    "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:builder-${ARCH}",
    "type=gha,scope=virtmcu-${ARCH}"
  ]
  cache-to = CI == "true" ? (
    PUSH_CACHE == "true" ? [
      "type=registry,ref=${REGISTRY}/${IMAGE_NAME_LOWER}/build-cache:runtime-${ARCH},mode=max"
    ] : [
      "type=gha,scope=virtmcu-${ARCH}"
    ]
  ) : []
}
