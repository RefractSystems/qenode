# Load registry and version configuration
VIRTMCU_IMAGE_REGISTRY ?= $(shell grep '^VIRTMCU_IMAGE_REGISTRY=' BUILD_DEPS | cut -d'=' -f2)
VIRTMCU_DEVENV_IMAGE   ?= $(shell grep '^VIRTMCU_DEVENV_IMAGE=' BUILD_DEPS | cut -d'=' -f2)
VIRTMCU_CI_IMAGE       ?= $(shell grep '^VIRTMCU_CI_IMAGE=' BUILD_DEPS | cut -d'=' -f2)

ARCH ?= $(shell uname -m | sed -e "s/x86_64/amd64/" -e "s/aarch64/arm64/")

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

# Prevent host-leaked VIRTUAL_ENV from breaking container builds.
# When opening this project in a Devcontainer via VS Code, the host OS's absolute
# VIRTUAL_ENV path (e.g., /Users/name/.../.venv) can leak into the container's
# environment. `uv sync --active` will try to write to this non-existent path
# and fail with "Permission denied". This defensively unsets invalid paths.
ifneq ($(VIRTUAL_ENV),)
ifeq ($(wildcard $(VIRTUAL_ENV)),)
$(warning Warning: VIRTUAL_ENV=$(VIRTUAL_ENV) does not exist (likely leaked from host). Unsetting it.)
unexport VIRTUAL_ENV
endif
endif

# Detection for container environment (system-wide Python mandate)
IN_CONTAINER := $(shell [ -f /.dockerenv ] || [ -f /run/.containerenv ] || [ "$$USER" = "vscode" ] && echo 1 || echo 0)
ifeq ($(IN_CONTAINER),1)
  UV_RUN_OPTS := --no-project
else
  # On bare metal, we still use uv but avoid workspace-local .venv if the mandate is "venv must go".
  # However, for local dev, --active is usually safer to avoid messing with system python.
  # If the user really wants it gone, they might be using a global uv environment.
  UV_RUN_OPTS := --active
endif

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
VIRTMCU_DOCKER_RUN_DEVENV_IMG = docker run --rm \
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

VIRTMCU_DOCKER_RUN_CI_IMG = docker run --rm \
	-v "$(CURDIR):/workspace" -w /workspace \
	-e HOST_UID=$$(id -u) \
	-e HOST_GID=$$(id -g) \
	-e PYTHONPATH=/workspace:/workspace/generated \
	-e CI=true \
	-e VIRTMCU_STALL_TIMEOUT_MS=120000 \
	-e VIRTMCU_USE_PREBUILT_QEMU=1

VIRTMCU_DOCKER_RUN_CI = $(VIRTMCU_DOCKER_RUN_CI_IMG) $(VIRTMCU_CI_IMG)
VIRTMCU_DOCKER_RUN_CI_ASAN = $(VIRTMCU_DOCKER_RUN_CI_IMG) $(VIRTMCU_CI_ASAN_IMG)

.PHONY: all build run clean clean-sim delete-profraw clean-debug distclean fmt-all fmt-python fmt-rust fmt-c fmt-meson fmt-yaml lint check-ffi build-test-artifacts build-tools install-git-hooks sync-versions check-versions docker-dev docker-all docker-base docker-toolchain docker-devenv docker-ci docker-ci-asan docker-runtime tag ensure-ci-image ensure-ci-asan-image
.PHONY: dev-unit ci-unit dev-integration ci-integration dev-integration-asan ci-integration-asan dev-unit-miri ci-unit-miri dev-unit-coverage ci-unit-coverage dev-integration-coverage ci-integration-coverage dev-peripheral-coverage ci-peripheral-coverage dev-lint ci-lint ci-local ci-check ci-full ci-build-third-party ci-build-third-party-asan

# Automatically determine the number of parallel jobs for make
JOBS ?= $(shell nproc 2>/dev/null || sysctl -n hw.logicalcpu 2>/dev/null || echo 4)

# By default, perform an incremental build
all: dev-all

# ------------------------------------------------------------------------------
# FFI Layout Verification
# ------------------------------------------------------------------------------

# Verify that Rust struct layouts match the QEMU binary ground truth.
check-ffi:
	@echo "==> Verifying FFI layouts..."
	@uv run $(UV_RUN_OPTS) python3 scripts/check-ffi.py

# ------------------------------------------------------------------------------
# Version Management
# ------------------------------------------------------------------------------

# Propagate versions from the BUILD_DEPS file to all downstream configuration files.
sync-versions:
	@echo "==> Synchronizing dependency versions..."
	@uv run $(UV_RUN_OPTS) python3 scripts/sync-versions.py
	@echo "✓ Versions synchronized."

# Verify that all versions are in sync across the codebase.
check-versions:
	@echo "==> Checking version synchronization..."
	@uv run $(UV_RUN_OPTS) python3 scripts/check-versions.py

# ------------------------------------------------------------------------------
# Build Targets
# ------------------------------------------------------------------------------

# Initialize the workspace: clone QEMU, apply all patches, and perform a full build.
# WARNING: This applies core patches that can trigger massive rebuilds. Run ONLY for first-time setup.
bootstrap:
	@bash scripts/install-third-party.sh

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
build-test-artifacts:
	@$(MAKE) -C tests/fixtures/guest_apps/boot_arm -j$(JOBS)
	@$(MAKE) -C tests/fixtures/guest_apps/uart_echo -j$(JOBS)
	@$(MAKE) -C tests/fixtures/guest_apps/telemetry_wfi -j$(JOBS)
	@$(MAKE) -C tests/fixtures/guest_apps/actuator -j$(JOBS)
	@$(MAKE) -C tests/fixtures/guest_apps/boot_riscv -j$(JOBS)
	@$(MAKE) -C tests/fixtures/guest_apps/flexray_bridge -j$(JOBS)
	@$(MAKE) -C tests/fixtures/guest_apps/spi_bridge -j$(JOBS)
	@$(MAKE) -C tests/fixtures/guest_apps/lin_bridge -j$(JOBS)
	@$(MAKE) -C tests/fixtures/guest_apps/complex_board -j$(JOBS)
	@$(MAKE) -C tests/fixtures/guest_apps/perf_bench -j$(JOBS)
	@if [ "$$CI" = "true" ] && command -v deterministic_coordinator >/dev/null 2>&1; then \
		echo "==> CI detected: Skipping Rust tools build (using pre-compiled binary in PATH)"; \
	else \
		echo "==> Building test tools (deterministic_coordinator, cyber_bridge, stress_adapter)..."; \
		ASAN_FLAG=$$([ "$$VIRTMCU_USE_ASAN" = "1" ] && echo "-Zsanitizer=address" || echo ""); \
		TSAN_FLAG=$$([ "$$VIRTMCU_USE_TSAN" = "1" ] && echo "-Zsanitizer=thread" || echo ""); \
		BOOTSTRAP=$$([ "$$VIRTMCU_USE_ASAN" = "1" ] || [ "$$VIRTMCU_USE_TSAN" = "1" ] && echo "1" || echo "0"); \
		RUSTFLAGS="$$ASAN_FLAG $$TSAN_FLAG"; \
		TRIPLE=$$(rustc -vV | grep "host:" | awk "{print \$$2}"); \
		RUSTC_BOOTSTRAP=$$BOOTSTRAP HOST_CFLAGS="" HOST_CXXFLAGS="" RUSTFLAGS="$$RUSTFLAGS" CARGO_BUILD_TARGET="$$TRIPLE" CARGO_TARGET_DIR="target$(BUILD_SUFFIX)" cargo build --release -j$(JOBS) -p zenoh_coordinator -p deterministic_coordinator -p cyber_bridge -p stress_adapter --target "$$TRIPLE"; \
	fi

# Build Python host orchestration tools
build-tools:
	@echo "==> Building virtmcu-tools package..."
	@cd packaging/virtmcu-tools && uv build >/dev/null && \
		WHEEL_FILE=$$(ls dist/*.whl | head -n 1) && \
		unzip -l "$$WHEEL_FILE" | grep "virtmcu_tools/repl2qemu/" >/dev/null && \
		unzip -l "$$WHEEL_FILE" | grep "virtmcu_tools/yaml2qemu.py" >/dev/null && \
		unzip -l "$$WHEEL_FILE" | grep "virtmcu_tools/mcp_server/" >/dev/null && \
		unzip -l "$$WHEEL_FILE" | grep "virtmcu_tools/qmp_bridge.py" >/dev/null && \
		echo "✓ virtmcu-tools package build passed."

# Launch the emulator using the test DTB and default arguments.
run:
	@bash target/release/virtmcu-run \
	  $(if $(wildcard tests/fixtures/guest_apps/boot_arm/minimal.dtb),--dtb tests/fixtures/guest_apps/boot_arm/minimal.dtb) \
	  $(if $(wildcard tests/fixtures/guest_apps/boot_arm/hello.elf),--kernel tests/fixtures/guest_apps/boot_arm/hello.elf) \
	  -nographic \
	  -m 128M \
	  $(EXTRA_ARGS)


# ------------------------------------------------------------------------------
# Python & Testing Targets (Unified dev/ci pairs)
# ------------------------------------------------------------------------------

# Configure Python environment using uv.
# In a container, this target is effectively a no-op that verifies the system install.
setup-python:
	@if [ "$(IN_CONTAINER)" = "1" ]; then \
		echo "==> Running in container: using system-wide Python environment."; \
		sudo uv pip install --system -r pyproject.toml -r requirements.txt --break-system-packages; \
	else \
		echo "==> Bare-metal detected: venv is BANNED in this project."; \
		echo "    Please use the provided DevContainer or manage your own system-wide dependencies."; \
		if ! command -v uv >/dev/null 2>&1; then \
			echo "    (uv is still recommended for tool execution)"; \
		fi; \
	fi


# ------------------------------------------------------------------------------
# Continuous Integration Targets (Docker/CI)
# ------------------------------------------------------------------------------

ensure-ci-image:
	@docker image inspect $(VIRTMCU_CI_IMG) >/dev/null 2>&1 || \
		(echo "==> Image $(VIRTMCU_CI_IMG) not found locally. Pulling..." && docker pull $(VIRTMCU_CI_IMG)) || \
		(echo "==> Pull failed. Building locally..." && bash scripts/docker-build.sh ci)

ensure-ci-asan-image:
	@docker image inspect $(VIRTMCU_CI_ASAN_IMG) >/dev/null 2>&1 || \
		(echo "==> Image $(VIRTMCU_CI_ASAN_IMG) not found locally. Pulling..." && docker pull $(VIRTMCU_CI_ASAN_IMG)) || \
		(echo "==> Pull failed. Building locally..." && bash scripts/docker-build.sh ci-asan)

ci-check: ensure-ci-image
	@$(VIRTMCU_DOCKER_RUN_CI) $(MAKE) dev-check

ci-lint: ensure-ci-image
	@$(VIRTMCU_DOCKER_RUN_CI) bash scripts/testing/run-lint.sh

ci-unit: ensure-ci-image
	@$(VIRTMCU_DOCKER_RUN_CI) bash scripts/testing/run-unit.sh

ci-unit-coverage: ensure-ci-image
	@$(VIRTMCU_DOCKER_RUN_CI) bash scripts/testing/run-unit-coverage.sh

ci-unit-miri: ensure-ci-image
	@$(VIRTMCU_DOCKER_RUN_CI) bash scripts/testing/run-unit-miri.sh

ci-integration: ensure-ci-image
	@if [ -z "$(DOMAIN)" ]; then \
		echo "❌ Error: DOMAIN is required for ci-integration."; \
		echo "==> Example: make ci-integration DOMAIN=boot_arm"; \
		exit 1; \
	fi
	@$(VIRTMCU_DOCKER_RUN_CI) bash scripts/testing/run-integration.sh $(DOMAIN)

ci-integration-coverage: ensure-ci-image
	@$(VIRTMCU_DOCKER_RUN_CI) bash scripts/testing/run-integration-coverage.sh

ci-integration-asan: ensure-ci-asan-image
	@echo "════════════════════════════════════════════════════"
	@echo "  CI ASan — Docker: ci-asan"
	@echo "════════════════════════════════════════════════════"
	@$(VIRTMCU_DOCKER_RUN_CI_ASAN) bash scripts/testing/run-integration-asan.sh
	@echo ""
	@echo "✓ ci-integration-asan passed."

ci-peripheral-coverage: ensure-ci-image
	@$(VIRTMCU_DOCKER_RUN_CI) bash scripts/testing/run-peripheral-coverage.sh


ci-build-third-party:
	@$(MAKE) third-party-builder

ci-build-third-party-asan:
	@VIRTMCU_USE_ASAN=1 $(MAKE) third-party-builder

# Run the full pipeline: ci-lint + ci-unit + ci-integration-asan + ci-unit-miri + all integration domains
ci-full: ensure-ci-image
	@echo ""
	@echo "════════════════════════════════════════════════════"
	@echo "  CI Full — Docker: ci"
	@echo "════════════════════════════════════════════════════"
	@echo ""
	@$(MAKE) ci-lint
	@$(MAKE) ci-unit
	@$(MAKE) ci-integration-asan
	@$(MAKE) ci-unit-miri
	@echo ""
	@echo "════════════════════════════════════════════════════"
	@echo "  CI Full — Integration smoke tests matrix (inside ci)"
	@echo "════════════════════════════════════════════════════"
	@mkdir -p coverage-data
	@$(VIRTMCU_DOCKER_RUN_CI_IMG) -e GCOV_PREFIX=/workspace/coverage-data -e GCOV_PREFIX_STRIP=3 $(VIRTMCU_CI_IMG) $(MAKE) dev-integration DOMAIN=all
	@echo ""
	@echo "════════════════════════════════════════════════════"
	@echo "  CI Full — Coverage Checks"
	@echo "════════════════════════════════════════════════════"
	@$(MAKE) ci-integration-coverage
	@$(MAKE) ci-peripheral-coverage
	@echo ""
	@echo "✓ ci-full passed."
# ------------------------------------------------------------------------------
# Development Targets (Local)
# ------------------------------------------------------------------------------

# --- General ---

# Setup developer environment: dependencies, version sync, and full build.
setup-dev: bootstrap sync-versions build-qemu

# Run the full development pipeline: build QEMU, build guest artifacts, lint, unit tests, integration tests, and peripheral coverage.
dev-all: build-qemu build-test-artifacts dev-check dev-integration dev-peripheral-coverage

# --- Linting ---
# Unified developer check: Lint + Unit Tests (Tier 1 parity)
dev-check: dev-lint dev-unit dev-unit-coverage

dev-lint: setup-python
	@bash scripts/testing/run-lint.sh

# --- Unit Tests ---
dev-unit: setup-python
	@cargo run -p virtmcu-test-runner --release -- run --tier unit

dev-unit-coverage: setup-python
	@bash scripts/testing/run-unit-coverage.sh

dev-unit-miri: setup-python
	@bash scripts/testing/run-unit-miri.sh

# --- Integration Tests ---
dev-integration: setup-python
	@uv run $(UV_RUN_OPTS) $(MAKE) build-test-artifacts
	cargo test -p native-integration

dev-integration-coverage: setup-python
	@bash scripts/testing/run-integration-coverage.sh

dev-integration-asan: setup-python
	@bash scripts/testing/run-integration-asan.sh

# Run host-side C coverage for peripheral plugins (inside ci)
dev-peripheral-coverage:
	@bash scripts/testing/run-peripheral-coverage.sh

# --- Git Hooks ---
install-git-hooks:
	@echo "==> Installing Git hooks..."
	@mkdir -p .git/hooks
	@printf '#!/bin/sh\nset -e\nmake dev-lint\n' > .git/hooks/pre-commit
	@printf '#!/bin/sh\nset -e\nmake dev-unit\n' > .git/hooks/pre-push
	@chmod +x .git/hooks/pre-push .git/hooks/pre-commit
	@echo "✓ Git hooks installed: pre-commit (lint) and pre-push (unit)."

# Aliases for backward compatibility
fmt-all: fmt-python fmt-rust fmt-meson fmt-c fmt-yaml

# Individual format targets
fmt-python: setup-python
	@echo "==> ruff format + fix..."
	@uv run $(UV_RUN_OPTS) ruff format .
	@uv run $(UV_RUN_OPTS) ruff check . --fix

fmt-rust:
	@echo "==> cargo fmt..."
	@cargo fmt --all

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
# All versions are read from the BUILD_DEPS file by scripts/docker-build.sh.
# Pass IMAGE_TAG=<tag> to override the local tag (default: dev).
#
#   make docker-dev    — base → toolchain → devenv with smoke tests (fast path)
#   make docker-all    — full pipeline including ci (~40 min)
#   make docker-base   — build a single stage (no smoke test, for debugging)

# Build docker base -> toolchain -> devenv with smoke tests
docker-dev:
	@bash scripts/docker-build.sh dev

# Build all docker stages including ci
docker-all:
	@bash scripts/docker-build.sh all

# Build only the docker base stage
docker-base:
	@bash scripts/docker-build.sh base

# Build only the docker toolchain stage
docker-toolchain:
	@bash scripts/docker-build.sh toolchain

# Build only the docker devenv stage
docker-devenv:
	@bash scripts/docker-build.sh devenv

# Build only the docker qemu-builder stage
third-party-builder:
	@bash scripts/docker-build.sh qemu-builder

# Build only the docker ci stage
docker-ci:
	@bash scripts/docker-build.sh ci

# Build only the docker ci-asan stage
docker-ci-asan:
	@bash scripts/docker-build.sh ci-asan

# Build only the docker runtime stage
docker-runtime:
	@bash scripts/docker-build.sh runtime

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
	@bash scripts/cleanup-sim.sh

# Remove backup and profile raw files.
delete-profraw:
	@echo "==> Deleting backup and profile raw files..."
	find . -type f \( -name "*~" -o -name "*profraw" \) -delete

# Alias for comprehensive cleanup of generated debugging and test artifacts.
clean-debug: clean

# Clean up Python artifacts, test binaries, and local tool builds.
# Note: This does NOT clean the QEMU build tree or remove downloaded sources.
clean:
	@echo "==> Cleaning generated files and test artifacts..."
	find . -name "*.pyc" -delete
	find . -name "__pycache__" -type d -exec rm -rf {} + 2>/dev/null || true
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
	rm -f .coverage
	rm -rf .pytest_cache .ruff_cache .hypothesis
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

# Serve the mdBook documentation locally (uses Python to avoid WebSocket/DevContainer port forwarding issues)
book-serve: book
	@echo "==> Serving mdBook..."
	@echo "    Click this link to open: http://localhost:8080"
	@python3 -m http.server -d target/book 8080
