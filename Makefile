# Load and export all variables from BUILD_DEPS
# This ensures that docker-bake.hcl has access to all version pins.
$(foreach var,$(shell grep -v '^#' BUILD_DEPS | grep -v '^[[:space:]]*$$'),$(eval export $(var)))

ARCH ?= $(shell uname -m | sed -e "s/x86_64/amd64/" -e "s/aarch64/arm64/")

# Calculate content-addressed tag for third-party images (QEMU + patches)
PATCHES_HASH := $(shell (cat BUILD_DEPS; find patches -type f | sort | xargs cat) | sha256sum | head -c 12)
export THIRD_PARTY_CACHE_TAG := $(QEMU_VERSION)-$(PATCHES_HASH)

# Dynamic IMAGE_TAG logic:
# 1. If explicitly provided via env/make args, use it.
# 2. If exactly on a git tag (e.g. v1.2.3), use the tag name (without the 'v').
# 3. If CI=true, use sha-<short_hash>.
# 4. Fallback to 'latest' to use the pre-built registry images.
ifndef IMAGE_TAG
  GIT_EXACT_TAG := $(shell git describe --tags --exact-match 2>/dev/null)
  ifneq ($(GIT_EXACT_TAG),)
    IMAGE_TAG := $(shell echo $(GIT_EXACT_TAG) | sed 's/^v//')
  else ifeq ($(CI),true)
    IMAGE_TAG := sha-$(shell git rev-parse --short HEAD 2>/dev/null || echo "unknown")
  else
    IMAGE_TAG := latest
  endif
endif

export IMAGE_TAG

# GitHub CI publishes multi-arch manifests, so we don't append -$(ARCH) for pulled images.
# Local builds via docker-build.sh will still tag with -$(ARCH) but we pull without it.
VIRTMCU_DEVENV_IMG ?= $(VIRTMCU_IMAGE_REGISTRY)/$(VIRTMCU_DEVENV_IMAGE):$(IMAGE_TAG)
VIRTMCU_CI_IMG ?= $(VIRTMCU_IMAGE_REGISTRY)/$(VIRTMCU_CI_IMAGE):$(IMAGE_TAG)
VIRTMCU_CI_ASAN_IMG ?= $(VIRTMCU_IMAGE_REGISTRY)/$(VIRTMCU_CI_IMAGE):$(IMAGE_TAG)-asan
VIRTMCU_USE_CCACHE ?= 0
export VIRTMCU_USE_CCACHE
export VIRTMCU_WORKSPACE := $(CURDIR)

# Detection for container environment
IN_CONTAINER := $(shell [ -f /.dockerenv ] || [ -f /run/.containerenv ] || [ "$$USER" = "vscode" ] && echo 1 || echo 0)

# Enable unstable cargo features for artifact dependencies (bindeps)
export CARGO_UNSTABLE_BINDEPS := true
export RUSTC_BOOTSTRAP := 1

ifeq ($(VIRTMCU_USE_ASAN),1)
  BUILD_SUFFIX := -asan
else ifeq ($(VIRTMCU_USE_TSAN),1)
  BUILD_SUFFIX := -tsan
else
  BUILD_SUFFIX :=
endif

# Environment configuration defaults
QEMU_SRC  ?= $(CURDIR)/third_party/qemu
QEMU_BUILD?= $(QEMU_SRC)/build-virtmcu$(BUILD_SUFFIX)

# Helper macros for running commands in Docker
VIRTMCU_DOCKER_RUN_DEVENV_IMG = docker run --rm --net=host \
	-e HOST_UID=$$(id -u) \
	-e HOST_GID=$$(id -g) \
	-e HOME=/home/vscode \
	-e USER=vscode \
	-e CI=true \
	-e CARGO_TARGET_DIR=/tmp/ci-target$(BUILD_SUFFIX) \
	-e VIRTMCU_SKIP_QEMU_HEADERS_WARNING=1 \
	-v "$(CURDIR):/workspace" \
	-v "$(CURDIR)/.ci-target$(BUILD_SUFFIX):/tmp/ci-target$(BUILD_SUFFIX)" \
	-v ci-cargo-registry:/usr/local/cargo/registry \
	-w /workspace

VIRTMCU_DOCKER_RUN_DEVENV = $(VIRTMCU_DOCKER_RUN_DEVENV_IMG) $(VIRTMCU_DEVENV_IMG)

VIRTMCU_DOCKER_RUN_CI_IMG = docker run --rm --net=host \
	-v "$(CURDIR):/workspace" -w /workspace \
	-v ci-cargo-registry:/usr/local/cargo/registry \
	-e HOST_UID=$$(id -u) \
	-e HOST_GID=$$(id -g) \
	-e HOME=/home/vscode \
	-e USER=vscode \
	-e CI=true \
	-e VIRTMCU_STALL_TIMEOUT_MS=120000 \
	-e VIRTMCU_USE_PREBUILT_QEMU=1

VIRTMCU_DOCKER_RUN_CI = $(VIRTMCU_DOCKER_RUN_CI_IMG) $(VIRTMCU_CI_IMG)
VIRTMCU_DOCKER_RUN_CI_ASAN = $(VIRTMCU_DOCKER_RUN_CI_IMG) $(VIRTMCU_CI_ASAN_IMG)

.PHONY: all build run clean clean-sim delete-profraw clean-debug distclean fmt-all fmt-rust fmt-c fmt-meson fmt-yaml lint build-test-artifacts install-git-hooks sync-versions docker-dev docker-all docker-base docker-toolchain docker-devenv docker-ci docker-ci-asan tag ensure-ci-image ensure-ci-asan-image
.PHONY: test-unit ci-unit test-integration ci-integration test-integration-asan ci-integration-asan test-unit-miri ci-unit-miri test-unit-coverage ci-unit-coverage test-integration-coverage ci-integration-coverage test-peripheral-coverage ci-peripheral-coverage test-lint ci-lint ci-local ci-check ci-full ci-build-third-party ci-build-third-party-asan build-test-runner

# Automatically determine the number of parallel jobs for make
JOBS ?= $(shell nproc 2>/dev/null || sysctl -n hw.logicalcpu 2>/dev/null || echo 4)
TRIPLE := $(shell rustc -vV | grep "host:" | awk '{print $$2}')

# Set Cargo environment variables for consistency across all targets
ifeq ($(VIRTMCU_USE_ASAN),1)
  export BUILD_SUFFIX := -asan
  export RUSTFLAGS := -Zsanitizer=address
else ifeq ($(VIRTMCU_USE_TSAN),1)
  export BUILD_SUFFIX := -tsan
  export RUSTFLAGS := -Zsanitizer=thread
else
  export BUILD_SUFFIX :=
  export RUSTFLAGS :=
endif

export CARGO_TARGET_DIR := $(CURDIR)/target$(BUILD_SUFFIX)
export CARGO_BUILD_JOBS := $(JOBS)
export CARGO_BUILD_TARGET := $(TRIPLE)

# Canonical paths for built artifacts
TARGET_BIN_DIR := $(CARGO_TARGET_DIR)/$(TRIPLE)/release
RUNNER_BIN     := $(TARGET_BIN_DIR)/virtmcu-test-runner

# Canonical Cargo command with unified flags
override CARGO_CMD := cargo +nightly
override CARGO_OPTS := -Z bindeps --target $(TRIPLE) -j$(JOBS)

# Canonical path to virtmcu-cli for sub-makefiles and guest-app builds.
# We use 'cargo run' for development to ensure it is always up to date.
export VIRTMCU_CLI := $(CARGO_CMD) run $(CARGO_OPTS) --manifest-path $(CURDIR)/Cargo.toml -p virtmcu-cli --release --

# By default, perform an incremental build
all: test-all  

# ------------------------------------------------------------------------------
# Version Management
# ------------------------------------------------------------------------------

# Propagate versions from the BUILD_DEPS file to all downstream configuration files.
sync-versions:
	@echo "==> Synchronizing dependency versions..."
	@$(CARGO_CMD) run $(CARGO_OPTS) -p virtmcu-cli -- setup sync-versions
	@echo "✓ Versions synchronized."

# ------------------------------------------------------------------------------
# Build Targets
# ------------------------------------------------------------------------------

# Initialize the workspace: clone QEMU, apply all patches, and perform a full build.
# WARNING: This applies core patches that can trigger massive rebuilds. Run ONLY for first-time setup.
bootstrap:
	@$(CARGO_CMD) run $(CARGO_OPTS) -p virtmcu-cli -- setup bootstrap


ifeq ($(VIRTMCU_USE_ASAN),1)
  ZENOHC_BUILD_DIR := third_party/zenoh-c-src/build-asan
else
  ZENOHC_BUILD_DIR := third_party/zenoh-c-src/build-release
endif
FLATCC_BUILD_DIR := third_party/flatcc-src/build

# Incremental build for QEMU and VirtMCU plugins
build-qemu:
	@echo "==> Rebuilding QEMU (jobs=$(JOBS))..."
	@$(MAKE) -C $(QEMU_BUILD) -j$(JOBS)
	@$(MAKE) -C $(QEMU_BUILD) install
	@echo "✓ Done."

# Incremental build for Zenoh-C
build-zenoh-c:
	@echo "==> Checking Zenoh-C build..."
	@cmake --build $(ZENOHC_BUILD_DIR) -j$(JOBS)
	@cmake --install $(ZENOHC_BUILD_DIR)

# Incremental build for FlatCC
build-flatcc:
	@echo "==> Checking FlatCC build..."
	@cmake --build $(FLATCC_BUILD_DIR) -j$(JOBS) --target install

# Check all third-party dependencies for updates
build-third-party: build-zenoh-c build-flatcc build-qemu

# Alias for backward compatibility
build: build-qemu

# Builds all test artifacts across all domains
GUEST_APP_DIRS := $(wildcard tests/fixtures/guest_apps/*)
GUEST_APP_TARGETS := $(foreach dir,$(GUEST_APP_DIRS),$(if $(wildcard $(dir)/Makefile),$(dir)))

.PHONY: $(GUEST_APP_TARGETS)
$(GUEST_APP_TARGETS):
	@$(MAKE) -C $@ -j$(JOBS)

build-test-artifacts: $(GUEST_APP_TARGETS)
	@if [ "$$CI" = "true" ] && command -v deterministic_coordinator >/dev/null 2>&1; then \
		echo "==> CI detected: Skipping Rust tools build (using pre-compiled binary in PATH)"; \
	else \
		echo "==> Building test tools (deterministic_coordinator, cyber_bridge, stress_adapter)..."; \
		$(CARGO_CMD) build --release $(CARGO_OPTS) -p zenoh_coordinator -p deterministic_coordinator -p cyber_bridge -p stress_adapter; \
	fi

# Launch the emulator using the test DTB and default arguments.
run:
	@$(TARGET_BIN_DIR)/virtmcu-run \
	  $(if $(wildcard tests/fixtures/guest_apps/boot_arm/minimal.dtb),--dtb tests/fixtures/guest_apps/boot_arm/minimal.dtb) \
	  $(if $(wildcard tests/fixtures/guest_apps/boot_arm/hello.elf),--kernel tests/fixtures/guest_apps/boot_arm/hello.elf) \
	  -nographic \
	  -m 128M \
	  $(EXTRA_ARGS)


# ------------------------------------------------------------------------------
# Continuous Integration Targets (Docker/CI)
# ------------------------------------------------------------------------------

ensure-ci-image:
	@docker image inspect $(VIRTMCU_CI_IMG) >/dev/null 2>&1 || \
		(echo "==> Image $(VIRTMCU_CI_IMG) not found locally. Pulling..." && docker pull $(VIRTMCU_CI_IMG)) || \
		(echo "==> Pull failed. Building locally..." && docker buildx bake ci --load)

ensure-ci-asan-image:
	@docker image inspect $(VIRTMCU_CI_ASAN_IMG) >/dev/null 2>&1 || \
		(echo "==> Image $(VIRTMCU_CI_ASAN_IMG) not found locally. Pulling..." && docker pull $(VIRTMCU_CI_ASAN_IMG)) || \
		(echo "==> Pull failed. Building locally..." && docker buildx bake ci-asan --load)

ci-check: ensure-ci-image
	@$(VIRTMCU_DOCKER_RUN_CI) $(MAKE) test-check

ci-lint: ensure-ci-image
	@$(VIRTMCU_DOCKER_RUN_CI) $(MAKE) test-lint

ci-unit: ensure-ci-image
	@$(VIRTMCU_DOCKER_RUN_CI) $(MAKE) test-unit

ci-unit-coverage: ensure-ci-image
	@$(VIRTMCU_DOCKER_RUN_CI) $(MAKE) test-unit-coverage

ci-unit-miri: ensure-ci-image
	@$(VIRTMCU_DOCKER_RUN_CI) $(MAKE) test-unit-miri

ci-integration: ensure-ci-image
	@$(VIRTMCU_DOCKER_RUN_CI) $(MAKE) test-integration DOMAIN=$(DOMAIN)

ci-integration-coverage: ensure-ci-image
	@$(VIRTMCU_DOCKER_RUN_CI) $(MAKE) test-integration-coverage

ci-integration-asan: ensure-ci-asan-image
	@$(VIRTMCU_DOCKER_RUN_CI_ASAN) $(MAKE) test-integration-asan DOMAIN=$(DOMAIN)

ci-peripheral-coverage: ensure-ci-image
	@$(VIRTMCU_DOCKER_RUN_CI) $(MAKE) test-peripheral-coverage


ci-build-third-party:
	@$(MAKE) third-party-builder

ci-build-third-party-asan:
	@VIRTMCU_USE_ASAN=1 $(MAKE) third-party-builder

# Run the full pipeline: ci-lint + ci-unit + ci-integration-asan + ci-unit-miri + all integration domains
ci-full: ensure-ci-image
	@$(MAKE) ci-lint
	@$(MAKE) ci-unit
	@$(MAKE) ci-integration-asan
	@$(MAKE) ci-unit-miri
	@mkdir -p coverage-data
	@$(VIRTMCU_DOCKER_RUN_CI_IMG) -e GCOV_PREFIX=/workspace/coverage-data -e GCOV_PREFIX_STRIP=3 $(VIRTMCU_CI_IMG) $(MAKE) test-integration DOMAIN=all
	@$(MAKE) ci-integration-coverage
	@$(MAKE) ci-peripheral-coverage

# ------------------------------------------------------------------------------
# Development Targets (Local)
# ------------------------------------------------------------------------------

# --- General ---

# Setup developer environment: dependencies, version sync, and full build.
setup-dev: bootstrap sync-versions build-qemu

# Run the full development pipeline: build QEMU, build guest artifacts, lint, unit tests, integration tests, and peripheral coverage.
test-all  : build-qemu build-test-artifacts test-check test-integration test-peripheral-coverage

# --- Linting ---
# Unified developer check: Lint + Unit Tests (Tier 1 parity)
test-check: test-lint test-unit test-unit-coverage

build-test-runner:
	@$(CARGO_CMD) build $(CARGO_OPTS) -p virtmcu-test-runner --release

test-lint: build-test-runner
	@$(RUNNER_BIN) lint

# --- Unit Tests ---
test-unit: build-test-runner
	@$(RUNNER_BIN) run --tier unit

test-unit-coverage: build-test-runner
	@$(RUNNER_BIN) coverage

test-unit-miri: build-test-runner
	@$(RUNNER_BIN) miri

# --- Integration Tests ---
test-integration: build-test-artifacts build-test-runner
	@$(RUNNER_BIN) run --tier integration $(if $(DOMAIN),--domain $(DOMAIN))

test-integration-coverage: build-test-artifacts build-test-runner
	@$(RUNNER_BIN) coverage --integration

# --- End-to-End Tests ---
test-e2e  :
	@echo "==> Running E2E tests..."
	@for script in tests/e2e/*.sh; do \
		echo "==> Running $$script"; \
		bash "$$script" || exit 1; \
	done

test-integration-asan: build-test-artifacts build-test-runner
	@$(RUNNER_BIN) run --tier integration --asan $(if $(DOMAIN),--domain $(DOMAIN))

# Run host-side C coverage for peripheral plugins (inside ci)
test-peripheral-coverage: build-test-runner
	@$(RUNNER_BIN) coverage --peripheral

# --- Git Hooks ---
install-git-hooks:
	@echo "==> Installing Git hooks..."
	@mkdir -p .git/hooks
	@printf '#!/bin/sh\nset -e\nmake test-lint\n' > .git/hooks/pre-commit
	@printf '#!/bin/sh\nset -e\nmake test-unit\n' > .git/hooks/pre-push
	@chmod +x .git/hooks/pre-push .git/hooks/pre-commit
	@echo "✓ Git hooks installed: pre-commit (lint) and pre-push (unit)."

# Aliases for backward compatibility
fmt-all: fmt-rust fmt-meson fmt-c fmt-yaml

# Individual format targets
fmt-rust:
	@echo "==> cargo fmt..."
	@cargo +nightly fmt --all

fmt-meson:
	@echo "==> meson format..."
	@meson fmt -i hw/meson.build && echo "✓ meson format passed." || { echo "❌ meson format failed"; exit 1; }

fmt-c:
	@echo "==> clang-format..."
	@find hw tools tests -type f \( -name "*.c" -o -name "*.h" -o -name "*.cpp" -o -name "*.cc" -o -name "*.hpp" \) \
		-not -path "*/rust/*" -not -path "*/third_party/*" -print0 | xargs -0 clang-format -i && echo "✓ clang-format passed." || { echo "❌ clang-format failed"; exit 1; }

fmt-yaml:
	@echo "==> stripping trailing whitespace from YAMLs..."
	@find . -type f \( -name "*.yml" -o -name "*.yaml" \) -not -path "*/third_party/*" -print0 | xargs -0 sed -i 's/[[:space:]]*$$//'

# ------------------------------------------------------------------------------
# Docker Image Targets
# ------------------------------------------------------------------------------
# All versions are read from the BUILD_DEPS file.
# Pass IMAGE_TAG=<tag> to override the local tag (default: latest).
#
#   make docker-dev    — base → toolchain → devenv with smoke tests (fast path)
#   make docker-all    — full pipeline including ci (~40 min)
#   make docker-base   — build a single stage (no smoke test, for debugging)

BAKE := docker buildx bake --load

# Build docker base -> toolchain -> devenv with smoke tests
docker-dev:
	@$(BAKE) base
	@$(CARGO_CMD) run $(CARGO_OPTS) --manifest-path xtask/Cargo.toml -- smoke-base
	@$(BAKE) toolchain
	@$(CARGO_CMD) run $(CARGO_OPTS) --manifest-path xtask/Cargo.toml -- smoke-toolchain
	@$(BAKE) devenv
	@$(CARGO_CMD) run $(CARGO_OPTS) --manifest-path xtask/Cargo.toml -- smoke-devenv
	@echo "✓ All dev-base stages built and verified."

# Build all docker stages including ci
docker-all: docker-dev
	@$(BAKE) third-party-base
	@$(BAKE) ci
	@$(CARGO_CMD) run $(CARGO_OPTS) --manifest-path xtask/Cargo.toml -- smoke-ci
	@$(BAKE) ci-asan
	@$(CARGO_CMD) run $(CARGO_OPTS) --manifest-path xtask/Cargo.toml -- smoke-ci-asan

# Build only the docker base stage
docker-base:
	@$(BAKE) base

# Build only the docker toolchain stage
docker-toolchain:
	@$(BAKE) toolchain

# Build only the docker devenv stage
docker-devenv:
	@$(BAKE) devenv

# Build only the docker qemu-builder stage (third-party-base in bake)
third-party-builder:
	@if [ "$(VIRTMCU_USE_ASAN)" = "1" ]; then \
		$(BAKE) third-party-base-asan; \
	else \
		$(BAKE) third-party-base; \
	fi

# Build only the docker ci stage
docker-ci:
	@$(BAKE) ci

# Build only the docker ci-asan stage
docker-ci-asan:
	@$(BAKE) ci-asan


# ------------------------------------------------------------------------------
# Release
# ------------------------------------------------------------------------------
# Create an annotated git tag, record the version in VERSION, and push both
# the commit and the tag.  GitHub CI then publishes versioned container images
# (virtmcu-devenv:vMAJOR.MINOR.PATCH, virtmcu-ci:vMAJOR.MINOR.PATCH, per-arch variants)
# and creates a GitHub Release with QEMU tarballs and the Python wheel.
#
# Usage:
#   make tag VERSION=v1.2.3
#
# Prerequisites: clean working tree, on the main branch, tag must not exist yet.

tag:
	@test -n "$(VERSION)" || (echo "Usage: make tag VERSION=v1.2.3" && exit 1)
	@echo "$(VERSION)" | grep -qE '^v[0-9]+\.[0-9]+\.[0-9]+$$' || \
		(echo "❌ VERSION must match vMAJOR.MINOR.PATCH (got: $(VERSION))" && exit 1)
	@test -z "$$(git status --porcelain)" || \
		(echo "❌ Working tree is dirty — commit or stash changes before releasing" && exit 1)
	@test "$$(git rev-parse --abbrev-ref HEAD)" = "main" || \
		(echo "❌ Releases must be tagged from the main branch" && exit 1)
	@git rev-parse $(VERSION) >/dev/null 2>&1 && \
		(echo "❌ Tag $(VERSION) already exists" && exit 1) || true
	@echo "$(VERSION)" | sed 's/^v//' > VERSION
	@git add VERSION
	@git commit -m "chore: release $(VERSION)"
	@git tag -a $(VERSION) -m "Release $(VERSION)"
	@git push origin main $(VERSION)
	@echo "✓ Tagged and pushed $(VERSION)"
	@echo "  CI will publish versioned images and create a GitHub Release automatically."

# ------------------------------------------------------------------------------
# Clean
# ------------------------------------------------------------------------------

# Kill all simulation-related processes and clean up temporary test files.
clean-sim:
	@$(CARGO_CMD) run $(CARGO_OPTS) -p virtmcu-cli -- setup cleanup-sim

# Remove backup and profile raw files.
delete-profraw:
	@echo "==> Deleting backup and profile raw files..."
	find . -type f \( -name "*~" -o -name "*profraw" \) -delete

# Alias for comprehensive cleanup of generated debugging and test artifacts.
clean-debug: clean

# Clean up test binaries, and local tool builds.
# Note: This does NOT clean the QEMU build tree or remove downloaded sources.
clean:
	@echo "==> Cleaning generated files and test artifacts..."
	find . -name "*.profraw" -delete
	find . -name "*.log" -delete
	find . -name "*.dtb" -not -path "./third_party/*" -delete
	find . -name "*.o" -not -path "./third_party/*" -delete
	find . -name "*.elf" -not -path "./tests/firmware/*" -not -path "./third_party/*" -delete
	find . -name "*.cli" -delete
	find . -name "*.arch" -delete
	find . -name "*.gcov" -delete
	find . -name "*.gcda" -delete
	find . -name "*.gcno" -not -path "./third_party/*" -delete
	find . -name "virtmcu-timeout-*" -delete
	find . -name "qmp-timeout-*" -delete
	rm -rf test-results/
	rm -rf tests/fixtures/guest_apps/*/results/
	rm -rf install/
	rm -f *_output.txt
	rm -f log.html report.html output.xml
	rm -rf tools/cyber_bridge/target
	rm -rf tools/systemc_adapter/build
	rm -rf tools/deterministic_coordinator/target
	rm -rf hw/rust/target
	rm -rf $(QEMU_SRC)/build-virtmcu/install
	rm -rf $(QEMU_SRC)/build-virtmcu-asan/install
	@echo "✓ Clean complete (QEMU sources remain)."

# Deep clean: completely remove downloaded sources and all artifacts.
# You will need to run 'make bootstrap' again after this.
distclean: clean
	rm -rf third_party
	rm -rf test-results
	rm -rf .ci-target*
	@echo "✓ Deep clean complete. Run 'make bootstrap' to rebuild the environment."
# ------------------------------------------------------------------------------
# Documentation
# ------------------------------------------------------------------------------

# Build the mdBook documentation (HTML + PDF)
book:
	@echo "==> Building mdBook (HTML + PDF)..."
	@if command -v mdbook >/dev/null 2>&1; then \
		if ! command -v mdbook-mermaid >/dev/null 2>&1; then echo "❌ mdbook-mermaid not installed."; exit 1; fi; \
		if ! command -v mdbook-pdf >/dev/null 2>&1; then echo "❌ mdbook-pdf not installed."; exit 1; fi; \
		mdbook build; \
	else \
		echo "❌ mdbook not installed. Please restart devcontainer or run: cargo install mdbook"; exit 1; \
	fi
	@echo "✓ mdBook built in target/book (HTML and PDF)."
	@mv target/book/pdf/output.pdf target/book/pdf/virtmcu_book.pdf

# Serve the mdBook documentation locally
book-serve:
	@echo "==> Serving mdBook..."
	@if command -v mdbook >/dev/null 2>&1; then \
		mdbook serve --port 8080; \
	else \
		echo "❌ mdbook not installed."; exit 1; \
	fi
