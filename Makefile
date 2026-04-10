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

.PHONY: all setup build run clean venv test test-unit test-robot test-all lint fmt install-hooks

# By default, perform an incremental build
all: build

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

# Create a Python virtual environment and install dependencies.
venv:
	python3 -m venv .venv
	.venv/bin/pip install --upgrade pip
	.venv/bin/pip install -r requirements.txt
	@echo "✓ Activate with: source .venv/bin/activate"

# Run integration smoke tests (Bash/QEMU level tests for phases 1 & 2)
test-integration:
	@echo "==> Running integration tests..."
	@for test_script in test/*/smoke_test.sh; do \
		echo "--> Running $$test_script"; \
		bash "$$test_script" || exit 1; \
	done
	@echo "✓ All integration tests passed."

# Run all Python unit tests (no QEMU required).
test: venv
	.venv/bin/python -m pytest tests/ -v

# Alias: same as test — explicit name for CI scripts.
test-unit: test

# Run Robot Framework integration tests (requires QEMU built via make setup).
test-robot: venv
	export PYTHONPATH=$(CURDIR) && \
	.venv/bin/robot \
	  --outputdir test-results/robot \
	  --loglevel INFO \
	  tests/test_qmp_keywords.robot \
	  tests/test_interactive_echo.robot

# Run the complete test suite: unit tests, integration smoke tests, Robot tests.
# Requires a built QEMU (run make setup first).
test-all: test test-integration test-robot

# ------------------------------------------------------------------------------
# Lint & Format
# ------------------------------------------------------------------------------

# Check Python style (same rules as CI).
lint:
	@echo "==> ruff check..."
	@ruff check tools/ tests/ patches/
	@echo "✓ Lint passed."

# Auto-fix formatting and fixable lint errors.
fmt:
	@echo "==> ruff format + fix..."
	@ruff format tools/ tests/ patches/
	@ruff check tools/ tests/ patches/ --fix
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

# Clean up Python artifacts and the virtual environment.
# Note: This does NOT clean the QEMU build tree.
clean:
	rm -rf .venv
	find . -name "*.pyc" -delete
	find . -name "__pycache__" -type d -exec rm -rf {} + 2>/dev/null || true
