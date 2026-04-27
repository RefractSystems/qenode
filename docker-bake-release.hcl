# docker-bake-release.hcl — Per-arch version tags on git tag releases.
#
# Included by ci-main.yml only when github.ref starts with refs/tags/.
# RELEASE_TAG is set to github.ref_name (e.g. v1.2.3) by the publish steps.
#
# IMPORTANT: In docker bake, when a target is redefined across multiple files,
# the 'tags' array is REPLACED, not merged. Therefore, we must explicitly
# include both the release version tag and the SHA-based tag (used by
# the manifest merge jobs) in this file.
#   devenv:v1.2.3-amd64      (lets users pull a specific arch+version directly)
#   devenv:sha-<sha>-amd64   (required by merge-devenv to assemble the manifest)

variable "REGISTRY" {
  default = "ghcr.io"
}

variable "IMAGE_NAME_LOWER" {
  default = "refractsystems/virtmcu"
}

variable "RELEASE_TAG" {
  default = ""
}

variable "IMAGE_TAG" {
  default = "dev"
}

variable "ARCH" {
  default = "amd64"
}

target "devenv" {
  tags = [
    "${REGISTRY}/${IMAGE_NAME_LOWER}/devenv:${RELEASE_TAG}-${ARCH}",
    "${REGISTRY}/${IMAGE_NAME_LOWER}/devenv:${IMAGE_TAG}-${ARCH}"
  ]
}

target "runtime" {
  tags = [
    "${REGISTRY}/${IMAGE_NAME_LOWER}/runtime:${RELEASE_TAG}-${ARCH}",
    "${REGISTRY}/${IMAGE_NAME_LOWER}/runtime:${IMAGE_TAG}-${ARCH}"
  ]
}
