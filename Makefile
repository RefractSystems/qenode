# ==============================================================================
# Top-level Makefile for virtmcu
#
# This Makefile provides convenient shorthand commands for common development 
# tasks. It delegates the actual heavy lifting to the shell scripts located 
# in the `scripts/` directory or to the QEMU build system.
#
# Most developers will only need:
#   make setup    — Clone QEMU, apply patches, and build from scratch (run once).
#   make          — Perform an incremental rebuild of QEMU after modifying `hw/`.
#   make run      — Launch QEMU using the minimal Phase 1 test DTB.
# ==============================================================================

# Environment configuration defaults
QEMU_SRC  ?= $(CURDIR)/third_party/qemu
QEMU_BUILD?= $(QEMU_SRC)/build-virtmcu
# Automatically determine the number of parallel jobs for make
JOBS      ?= $(shell nproc 2>/dev/null || sysctl -n hw.logicalcpu 2>/dev/null || echo 4)

.PHONY: all setup build run clean distclean venv test test-unit test-robot test-all lint fmt install-hooks sync-versions

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

# ------------------------------------------------------------------------------
# Build Targets
# ------------------------------------------------------------------------------

# Initialize the workspace: clone QEMU, apply all patches, and perform a full build.
setup:
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
	@echo "==> Building test artifacts..."
	@$(MAKE) -C test/phase1
	@$(MAKE) -C test/phase8
	@$(MAKE) -C test/phase12
	@$(MAKE) -C test/actuator
	@$(MAKE) -C test/riscv
	@echo "==> Running integration tests..."
	@for test_script in test/*/smoke_test.sh; do \
		echo "--> Running $$test_script"; \
		bash "$$test_script" || exit 1; \
	done
	@echo "✓ All integration tests passed."

# Alias for running all phase smoke tests in one go
smoke-tests: test-integration

# Run all Python unit tests (no QEMU required).
test: venv
	uv run pytest tests/ -v

# Alias: same as test — explicit name for CI scripts.
test-unit: test

# Run Robot Framework integration tests (requires QEMU built via make setup).
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
	@./scripts/run.sh --dtb test/phase1/minimal.dtb --kernel test/phase1/hello.elf \
	  -display none -plugin third_party/qemu/build-virtmcu/contrib/plugins/libdrcov.so,filename=hello.drcov -d plugin
	@uv run python3 tools/analyze_coverage.py hello.drcov test/phase1/hello.elf --fail-under 80
	@echo "✓ Guest coverage check passed."

# Generate host-side C/Rust coverage report (requires lcov)
coverage-report:
	@echo "==> Generating host-side coverage report..."
	@mkdir -p test-results/coverage
	lcov --quiet --capture --directory $(QEMU_BUILD) --output-file test-results/coverage/host.info --rc lcov_branch_coverage=1
	lcov --quiet --extract test-results/coverage/host.info "*/hw/virtmcu/*" --output-file test-results/coverage/host_filtered.info --rc lcov_branch_coverage=1
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
# Lint & Format
# ------------------------------------------------------------------------------

# Check Python style (same rules as CI).
lint: venv
	@echo "==> ruff check..."
	uv run ruff check tools/ tests/ patches/
	@echo "✓ Lint passed."

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
# Clean
# ------------------------------------------------------------------------------

# Clean up Python artifacts, test binaries, and local tool builds.
# Note: This does NOT clean the QEMU build tree or remove downloaded sources.
clean:
	find . -name "*.pyc" -delete
	find . -name "__pycache__" -type d -exec rm -rf {} + 2>/dev/null || true
	-$(MAKE) -C test/phase1 clean
	-$(MAKE) -C test/phase8 clean
	rm -rf tools/cyber_bridge/build
	rm -rf tools/systemc_adapter/build
	rm -rf tools/zenoh_coordinator/target
	@echo "✓ Clean complete (QEMU sources and .venv remain)."

# Deep clean: completely remove downloaded sources, virtual environments, and all artifacts.
# You will need to run 'make setup' again after this.
distclean: clean
	rm -rf .venv
	rm -rf third_party
	rm -rf test-results
	@echo "✓ Deep clean complete. Run 'make setup' to rebuild the environment."
