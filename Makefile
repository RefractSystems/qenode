# qenode top-level Makefile
#
# Delegates heavy lifting to scripts/. Most developers only need:
#   make setup    — clone QEMU, apply patches, build (run once)
#   make          — rebuild QEMU after changing hw/ sources
#   make run      — launch QEMU with a minimal test DTB

QEMU_SRC  ?= $(HOME)/src/qemu
QEMU_BUILD?= $(QEMU_SRC)/build-qenode
JOBS      ?= $(shell nproc 2>/dev/null || sysctl -n hw.logicalcpu 2>/dev/null || echo 4)

.PHONY: all setup build run clean venv test

# Default: rebuild QEMU (fast — only changed files recompile)
all: build

## setup: clone QEMU, apply patch series, configure, full build (run once)
setup:
	@bash scripts/setup-qemu.sh

## build: incremental rebuild after changing hw/ sources
build:
	@echo "==> Rebuilding QEMU (jobs=$(JOBS))..."
	@$(MAKE) -C $(QEMU_SRC) -j$(JOBS)
	@$(MAKE) -C $(QEMU_SRC) install
	@echo "✓ Done."

## run: launch QEMU with the minimal Phase 1 test DTB (if it exists)
run:
	@bash scripts/run.sh \
	  -M arm-generic-fdt \
	  -nographic \
	  -m 128M \
	  $(if $(wildcard tests/phase1/minimal.dtb),-hw-dtb tests/phase1/minimal.dtb,) \
	  $(EXTRA_ARGS)

## venv: create Python virtual environment and install dependencies
venv:
	python3 -m venv .venv
	.venv/bin/pip install --upgrade pip
	.venv/bin/pip install -r requirements.txt
	@echo "✓ Activate with: source .venv/bin/activate"

## test: run Python unit tests
test: venv
	.venv/bin/python -m pytest tests/ -v

## clean: remove generated files (does not touch QEMU source tree)
clean:
	rm -rf .venv
	find . -name "*.pyc" -delete
	find . -name "__pycache__" -type d -exec rm -rf {} + 2>/dev/null || true
