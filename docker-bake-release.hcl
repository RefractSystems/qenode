# docker-bake-release.hcl — Per-arch version tags on git tag releases.
#
# Included by ci-main.yml only when github.ref starts with refs/tags/.
# RELEASE_TAG is set to github.ref_name (e.g. v1.2.3) by the publish steps.
#
# In docker bake, tag arrays from multiple files are merged, not replaced.
# So these tags are ADDED to the SHA-based tags from docker-bake.hcl, giving
# each per-arch image both identifiers:
#   devenv:sha-<sha>-amd64   (used by merge-devenv to assemble the manifest)
#   devenv:v1.2.3-amd64      (lets users pull a specific arch+version directly)

variable "REGISTRY" {
  default = "ghcr.io"
}

variable "IMAGE_NAME_LOWER" {
  default = "refractsystems/virtmcu"
}

variable "RELEASE_TAG" {
  default = ""
}

variable "ARCH" {
  default = "amd64"
}

target "devenv" {
  tags = ["${REGISTRY}/${IMAGE_NAME_LOWER}/devenv:${RELEASE_TAG}-${ARCH}"]
}

target "runtime" {
  tags = ["${REGISTRY}/${IMAGE_NAME_LOWER}/runtime:${RELEASE_TAG}-${ARCH}"]
}
