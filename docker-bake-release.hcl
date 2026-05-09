# docker-bake-release.hcl — Per-arch version tags on git tag releases.
#
# Included by ci-main.yml only when github.ref starts with refs/tags/.
# RELEASE_TAG is set to github.ref_name (e.g. v1.2.3) by the publish steps.
#
# IMPORTANT: In docker bake, when a target is redefined across multiple files,
# the 'tags' array is REPLACED, not merged. Therefore, we must explicitly
# include both the release version tag and the SHA-based tag (used by
# the manifest merge jobs) in this file.
#   ${VIRTMCU_DEVENV_IMAGE}:v1.2.3-amd64      (lets users pull a specific arch+version directly)
#   ${VIRTMCU_DEVENV_IMAGE}:sha-<sha>-amd64   (required by merge-devenv to assemble the manifest)

variable "VIRTMCU_IMAGE_REGISTRY" {}
variable "VIRTMCU_DEVENV_IMAGE" {}
variable "VIRTMCU_CI_IMAGE" {}

variable "RELEASE_TAG" {
  default = ""
}

variable "IMAGE_TAG" {
  default = "dev"
}

variable "ARCH" {
  default = "amd64"
}

target "base" {
  tags = [
    "${VIRTMCU_IMAGE_REGISTRY}/base:${RELEASE_TAG}-${ARCH}",
    "${VIRTMCU_IMAGE_REGISTRY}/base:${IMAGE_TAG}-${ARCH}"
  ]
}

target "toolchain" {
  tags = [
    "${VIRTMCU_IMAGE_REGISTRY}/toolchain:${RELEASE_TAG}-${ARCH}",
    "${VIRTMCU_IMAGE_REGISTRY}/toolchain:${IMAGE_TAG}-${ARCH}"
  ]
}

target "devenv" {
  tags = [
    "${VIRTMCU_IMAGE_REGISTRY}/${VIRTMCU_DEVENV_IMAGE}:${RELEASE_TAG}-${ARCH}",
    "${VIRTMCU_IMAGE_REGISTRY}/${VIRTMCU_DEVENV_IMAGE}:${IMAGE_TAG}-${ARCH}"
  ]
}

target "ci" {
  tags = [
    "${VIRTMCU_IMAGE_REGISTRY}/${VIRTMCU_CI_IMAGE}:${RELEASE_TAG}-${ARCH}",
    "${VIRTMCU_IMAGE_REGISTRY}/${VIRTMCU_CI_IMAGE}:${IMAGE_TAG}-${ARCH}"
  ]
}

