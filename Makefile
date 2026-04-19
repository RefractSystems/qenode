# ==============================================================================
# Top-level Makefile for virtmcu
#
# This Makefile provides convenient shorthand commands for common development 
# tasks. It delegates the actual heavy lifting to the shell scripts located 
# in the `scripts/` directory or to the QEMU build system.
#
# Most developers will only need:
#   make setup-initial — Clone QEMU, apply patches, and build from scratch (run ONLY once per environment).
#   make build         — Perform an incremental rebuild of QEMU after modifying `hw/`. (Default target)
#   make run      — Launch QEMU using the minimal Phase 1 test DTB.
# ==============================================================================

# Environment configuration defaults
QEMU_SRC  ?= $(CURDIR)/third_party/qemu
QEMU_BUILD?= $(QEMU_SRC)/build-virtmcu
# Automatically determine the number of parallel jobs for make
JOBS      ?= $(shell nproc 2>/dev/null || sysctl -n hw.logicalcpu 2>/dev/null || echo 4)

.PHONY: all setup-initial build run clean clean-sim clean-debug distclean venv test test-unit test-robot test-all lint fmt install-hooks sync-versions check-versions docker-dev docker-all docker-base docker-toolchain docker-devenv docker-builder docker-runtime ci-local ci-full

# By default, perform an incremental build
all: build

# ------------------------------------------------------------------------------
# Version Management
# ------------------------------------------------------------------------------

# Propagate versions from the VERSIONS file to all downstream configuration files.
sync-versions:
	@echo "==> Synchronizing dependency versions..."
	@python3 scripts/sync-versions.py
	@echo "✓ Versions synchronized."

# Verify that all versions are in sync across the codebase.
check-versions:
	@echo "==> Checking version synchronization..."
	@python3 scripts/check-versions.py

# ------------------------------------------------------------------------------
# Build Targets
# ------------------------------------------------------------------------------

# Initialize the workspace: clone QEMU, apply all patches, and perform a full build.
# WARNING: This applies core patches that can trigger massive rebuilds. Run ONLY for first-time setup.
setup-initial:
	@bash scripts/setup-qemu.sh

# Incremental rebuild: useful when you only modify files in the `hw/` directory.
build:
	@echo "==> Rebuilding QEMU (jobs=$(JOBS))..."
	@$(MAKE) -C $(QEMU_BUILD) -j$(JOBS)
	@$(MAKE) -C $(QEMU_BUILD) install
	@echo "✓ Done."

# Launch the emulator using the test DTB and default arguments.
run:
	@bash scripts/run.sh \
	  $(if $(wildcard test/phase1/minimal.dtb),--dtb test/phase1/minimal.dtb) \
	  $(if $(wildcard test/phase1/hello.elf),--kernel test/phase1/hello.elf) \
	  -nographic \
	  -m 128M \
	  $(EXTRA_ARGS)

# ------------------------------------------------------------------------------
# Python & Testing Targets
# ------------------------------------------------------------------------------

# Create a Python virtual environment and install dependencies using uv.
venv:
	@if ! command -v uv >/dev/null 2>&1; then \
		echo "==> uv not found, installing..."; \
		curl -LsSf https://astral.sh/uv/install.sh | sh; \
		export PATH="$$HOME/.local/bin:$$PATH"; \
	fi
	uv sync
	@echo "✓ Virtual environment synchronized with uv."
	@echo "✓ Activate with: source .venv/bin/activate"

# Run integration smoke tests (Bash/QEMU level tests for phases 1 & 2)
test-integration: venv
	@bash scripts/cleanup-sim.sh --quiet
	@echo "==> Building test artifacts..."
	@$(MAKE) -C test/phase1
	@$(MAKE) -C test/phase8
	@$(MAKE) -C test/phase12
	@$(MAKE) -C test/actuator
	@$(MAKE) -C test/riscv
	@echo "==> Running integration tests..."
	@for test_script in test/*/smoke_test.sh; do \
		echo "--> Running $$test_script"; \
		bash "$$test_script" || { bash scripts/cleanup-sim.sh; exit 1; }; \
		bash scripts/cleanup-sim.sh --quiet; \
	done
	@echo "✓ All integration tests passed."

# Alias for running all phase smoke tests in one go
smoke-tests: test-integration

# Run all Python unit tests (no QEMU required).
test: venv
	PYTHONPATH=$(CURDIR) uv run pytest tests/ -v

# Alias: same as test — explicit name for CI scripts.
test-unit: test

# Run Robot Framework integration tests (requires QEMU built via make setup-initial).
test-robot: venv
	export PYTHONPATH=$(CURDIR) && \
	uv run robot \
	  --outputdir test-results/robot \
	  --loglevel INFO \
	  tests/test_qmp_keywords.robot \
	  tests/test_interactive_echo.robot

# Run guest firmware coverage analysis (Phase 1)
test-coverage-guest: build-test-artifacts
	@echo "==> Running guest firmware coverage (drcov)..."
	uv run python3 -m pyelftools --version >/dev/null 2>&1 || uv pip install pyelftools
	@DRCOV_SO=$$(find third_party/qemu -name "libdrcov.so" 2>/dev/null | head -n 1); \
	timeout 5s ./scripts/run.sh --dtb test/phase1/minimal.dtb --kernel test/phase1/hello.elf \
	  -display none -plugin "$$DRCOV_SO",filename=hello.drcov -d plugin || true
	@uv run python3 tools/analyze_coverage.py hello.drcov test/phase1/hello.elf --fail-under 80
	@echo "✓ Guest coverage check passed."

# Generate host-side C/Rust coverage report (requires lcov)
coverage-report:
	@echo "==> Generating host-side coverage report..."
	@mkdir -p test-results/coverage
	lcov --quiet --capture \
		--directory $(QEMU_BUILD)/libhw-virtmcu-dummy.a.p \
		--directory $(QEMU_BUILD)/libhw-virtmcu-mmio-socket-bridge.a.p \
		--directory $(QEMU_BUILD)/libhw-virtmcu-remote-port-bridge.a.p \
		--directory $(QEMU_BUILD)/libhw-virtmcu-rust-dummy.a.p \
		--directory $(QEMU_BUILD)/libhw-virtmcu-zenoh.a.p \
		--output-file test-results/coverage/host.info --rc branch_coverage=1 --ignore-errors empty
	lcov --quiet --extract test-results/coverage/host.info "*/hw/virtmcu/*" --output-file test-results/coverage/host_filtered.info --rc branch_coverage=1
	genhtml --quiet test-results/coverage/host_filtered.info --output-directory test-results/coverage/html --title "virtmcu Host Coverage" --legend --branch-coverage
	@echo "✓ Report generated: test-results/coverage/html/index.html"

build-test-artifacts:
	@$(MAKE) -C test/phase1
	@$(MAKE) -C test/phase8
	@$(MAKE) -C test/phase12
	@$(MAKE) -C test/actuator
	@$(MAKE) -C test/riscv

# Run the complete test suite: unit tests, integration smoke tests, Robot tests.
test-all: test test-integration test-robot test-coverage-guest

# ------------------------------------------------------------------------------
# Continuous Integration (CI) - Parity with GitHub Actions
# ------------------------------------------------------------------------------
# This target guarantees that local CI passes only if the GitHub Actions CI
# and the GHCR Docker Publish workflows will also pass.
ci-local: lint test-all
	@echo "==> Verifying Docker Publish (GitHub Actions parity)..."
	@bash scripts/docker-build.sh devenv
	@bash scripts/docker-build.sh runtime
	@echo "✓ Local CI complete! Code and Docker artifacts are ready for GitHub."

# ------------------------------------------------------------------------------
# Lint & Format
# ------------------------------------------------------------------------------

# Check Python style (same rules as CI).
lint: venv check-versions lint-cargo
	@echo "==> ruff check..."
	uv run ruff check tools/ tests/ patches/
	@echo "✓ Lint passed."

# Check Cargo workspace versions.
lint-cargo:
	@echo "==> Checking Cargo workspace version synchronization..."
	@cd hw/rust && cargo metadata --no-deps --format-version 1 | \
		python3 -c "import sys,json; m=json.load(sys.stdin); vs=set(p['version'] for p in m['packages']); assert len(vs)==1, f'version drift: {vs}'"
	@echo "✓ Cargo workspace versions aligned."
	@echo "==> Running cargo fmt --check..."
	@cd hw/rust && cargo fmt --all --check
	@echo "==> Running cargo clippy..."
	@cd hw/rust && cargo clippy --workspace -- -D warnings -D clippy::all

# Auto-fix formatting and fixable lint errors.
fmt: venv
	@echo "==> ruff format + fix..."
	uv run ruff format tools/ tests/ patches/
	uv run ruff check tools/ tests/ patches/ --fix
	@echo "✓ Done."

# Install a git pre-push hook that runs lint before any push.
# Opt-in: run once with  make install-hooks
install-hooks:
	@echo "==> Installing pre-push hook..."
	@mkdir -p .git/hooks
	@printf '#!/bin/sh\nset -e\nmake lint\n' > .git/hooks/pre-push
	@chmod +x .git/hooks/pre-push
	@echo "✓ pre-push hook installed (make lint will run on every push)."

# ------------------------------------------------------------------------------
# Local CI Simulation
# Mirrors the GitHub CI tier structure so you can validate locally before push.
#
#   make ci-local   — Tier 1 (lint + check-versions + unit tests) +
#                     Tier 2 (base → toolchain → devenv Docker builds with smoke tests)
#                     ~10 min with warm Docker cache; ~15 min cold.
#                     Covers every CI check that does NOT require the full QEMU build.
#
#   make ci-full    — ci-local + builder stage (QEMU compile, ~40 min cold) +
#                     runtime image + representative phase smoke tests inside container.
#                     This is the closest local equivalent to the full GitHub CI run.
#
# Gaps vs GitHub CI (tools not installed locally):
#   shellcheck — install with: brew install shellcheck
#   hadolint   — install with: brew install hadolint
# Both are skipped with a warning if absent; install them for full parity.
# ------------------------------------------------------------------------------

ci-local: venv check-versions
	@echo ""
	@echo "════════════════════════════════════════════════════"
	@echo "  CI Tier 1 — Lint"
	@echo "════════════════════════════════════════════════════"
	@echo "==> ruff check..."
	uv run ruff check tools/ tests/ patches/
	@echo "==> shellcheck..."
	@if command -v shellcheck >/dev/null 2>&1; then \
		shellcheck --severity=warning scripts/*.sh; \
		echo "✓ shellcheck passed."; \
	else \
		echo "  WARNING: shellcheck not installed (brew install shellcheck) — skipping."; \
	fi
	@echo "==> hadolint..."
	@if command -v hadolint >/dev/null 2>&1; then \
		hadolint --ignore DL3008 --ignore DL3009 --ignore DL4006 --ignore SC2016 --ignore SC2015 --ignore DL3002 --ignore DL3016 docker/Dockerfile; \
		echo "✓ hadolint passed."; \
	else \
		echo "  WARNING: hadolint not installed (brew install hadolint) — skipping."; \
	fi
	@echo ""
	@echo "════════════════════════════════════════════════════"
	@echo "  CI Tier 1 — Unit Tests (no QEMU)"
	@echo "════════════════════════════════════════════════════"
	PYTHONPATH=$(CURDIR) uv run pytest \
		tests/repl2qemu/ \
		-v --tb=short
	@echo ""
	@echo "════════════════════════════════════════════════════"
	@echo "  CI Tier 2 — Docker: base → toolchain → devenv"
	@echo "════════════════════════════════════════════════════"
	@bash scripts/docker-build.sh dev
	@echo ""
	@echo "✓ ci-local passed."
	@echo "  To run the full pipeline (builder ~40 min + integration tests): make ci-full"

ci-full: ci-local
	@echo ""
	@echo "════════════════════════════════════════════════════"
	@echo "  CI Full — Docker: builder + runtime"
	@echo "════════════════════════════════════════════════════"
	@bash scripts/docker-build.sh builder
	@bash scripts/docker-build.sh runtime
	@echo ""
	@echo "════════════════════════════════════════════════════"
	@echo "  CI Full — Integration smoke tests (inside builder)"
	@echo "════════════════════════════════════════════════════"
	@echo "  Running Phase 1 (ARM bare-metal boot)..."
	docker run --rm \
		-v "$(CURDIR):/workspace" -w /workspace \
		-e PYTHONPATH=/workspace \
		-e VIRTMCU_STALL_TIMEOUT_MS=60000 \
		virtmcu-builder:dev \
		bash -c "make -C test/phase1 && bash test/phase1/smoke_test.sh"
	@echo "  Running pytest unit tests inside builder..."
	docker run --rm \
		-v "$(CURDIR):/workspace" -w /workspace \
		-e PYTHONPATH=/workspace \
		virtmcu-builder:dev \
		bash -c "uv pip install --system --break-system-packages -r pyproject.toml && \
		         python3 -m pytest tests/repl2qemu/ tests/test_yaml2qemu.py \
		                        tests/test_cli_generator.py tests/test_fdt_emitter.py \
		                        -v --tb=short"
	@echo ""
	@echo "✓ ci-full passed."
	@echo "  NOTE: Phase 4-16 smoke tests, pytest-qmp, and Robot Framework"
	@echo "  require additional test artifacts. Run them individually:"
	@echo "    docker run --rm -v \"\$$(pwd):/workspace\" -w /workspace -e PYTHONPATH=/workspace \\"
	@echo "      virtmcu-builder:dev bash -c \"<pre> && bash test/phaseN/smoke_test.sh\""

# ------------------------------------------------------------------------------
# Docker Image Targets
# ------------------------------------------------------------------------------
# All versions are read from the VERSIONS file by scripts/docker-build.sh.
# Pass IMAGE_TAG=<tag> to override the local tag (default: dev).
#
#   make docker-dev    — base → toolchain → devenv with smoke tests (fast path)
#   make docker-all    — full pipeline including builder (~40 min) and runtime
#   make docker-base   — build a single stage (no smoke test, for debugging)

docker-dev:
	@bash scripts/docker-build.sh dev

docker-all:
	@bash scripts/docker-build.sh all

docker-base:
	@bash scripts/docker-build.sh base

docker-toolchain:
	@bash scripts/docker-build.sh toolchain

docker-devenv:
	@bash scripts/docker-build.sh devenv

docker-builder:
	@bash scripts/docker-build.sh builder

docker-runtime:
	@bash scripts/docker-build.sh runtime

# ------------------------------------------------------------------------------
# Clean
# ------------------------------------------------------------------------------

# Kill all simulation-related processes and clean up temporary test files.
clean-sim:
	@bash scripts/cleanup-sim.sh

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
	find . -name "virtmcu-timeout-*" -delete
	find . -name "qmp-timeout-*" -delete
	rm -f .coverage
	rm -rf .pytest_cache .ruff_cache
	rm -rf test-results/
	rm -rf test/*/results/
	rm -rf install/
	rm -f *_output.txt
	rm -f log.html report.html output.xml
	rm -rf tools/cyber_bridge/build
	rm -rf tools/systemc_adapter/build
	rm -rf tools/zenoh_coordinator/target
	rm -rf hw/rust/target
	@echo "✓ Clean complete (QEMU sources and .venv remain)."

# Deep clean: completely remove downloaded sources, virtual environments, and all artifacts.
# You will need to run 'make setup-initial' again after this.
distclean: clean
	rm -rf .venv
	rm -rf third_party
	rm -rf test-results
	@echo "✓ Deep clean complete. Run 'make setup-initial' to rebuild the environment."
