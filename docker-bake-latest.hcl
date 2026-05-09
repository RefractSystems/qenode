# docker-bake-latest.hcl — Adds :latest tags when on main branch
#
# IMPORTANT: In docker bake, when a target is redefined, the 'tags' array
# is REPLACED, not merged. We must include both 'latest' and the specific
# 'sha-<sha>' tags to ensure manifest merge jobs still work.

variable "VIRTMCU_IMAGE_REGISTRY" {}
variable "VIRTMCU_DEVENV_IMAGE" {}
variable "VIRTMCU_CI_IMAGE" {}

variable "IMAGE_TAG" {
  default = "latest"
}

variable "ARCH" {
  default = "amd64"
}

target "base" {
  tags = [
    "${VIRTMCU_IMAGE_REGISTRY}/base:latest-${ARCH}",
    "${VIRTMCU_IMAGE_REGISTRY}/base:${IMAGE_TAG}-${ARCH}"
  ]
}

target "toolchain" {
  tags = [
    "${VIRTMCU_IMAGE_REGISTRY}/toolchain:latest-${ARCH}",
    "${VIRTMCU_IMAGE_REGISTRY}/toolchain:${IMAGE_TAG}-${ARCH}"
  ]
}

target "devenv" {
  tags = [
    "${VIRTMCU_IMAGE_REGISTRY}/${VIRTMCU_DEVENV_IMAGE}:latest-${ARCH}",
    "${VIRTMCU_IMAGE_REGISTRY}/${VIRTMCU_DEVENV_IMAGE}:${IMAGE_TAG}-${ARCH}"
  ]
}


