ARCH ?= $(shell uname -m | sed -e "s/x86_64/amd64/" -e "s/aarch64/arm64/")
IMAGE_TAG ?= dev
DEVENV_BASE_IMG ?= ghcr.io/refractsystems/virtmcu/devenv-base:$(IMAGE_TAG)-$(ARCH)
BUILDER_IMG ?= ghcr.io/refractsystems/virtmcu/builder:$(IMAGE_TAG)-$(ARCH)
CARGO_CACHE_DIR ?= $(CURDIR)/.cargo-cache
VIRTMCU_USE_CCACHE ?= 0
export VIRTMCU_USE_CCACHE

# ==============================================================================
# Top-level Makefile for virtmcu
#
# This Makefile provides convenient shorthand commands for common development 
# tasks. It delegates the actual heavy lifting to the shell scripts located 
# in the `scripts/` directory or to the QEMU build system.
#
# Most developers will only need:
#   make setup-initial — Clone QEMU, apply patches, and build from scratch (run ONLY once per environment).
#   make build   — Perform an incremental rebuild of QEMU after modifying `hw/`. (Default target)
#   make run      — Launch QEMU using the minimal Phase 1 test DTB.
# ==============================================================================

# Environment configuration defaults
QEMU_SRC  ?= $(CURDIR)/third_party/qemu
QEMU_BUILD?= $(QEMU_SRC)/build-virtmcu
# Automatically determine the number of parallel jobs for make
JOBS      ?= $(shell nproc 2>/dev/null || sysctl -n hw.logicalcpu 2>/dev/null || echo 4)

.PHONY: all setup-initial build run clean clean-sim clean-debug distclean venv fmt fmt-python fmt-rust fmt-c fmt-meson fmt-yaml lint lint-python lint-python-types lint-rust lint-c lint-shell lint-docker lint-yaml lint-actions lint-meson lint-spelling check-ffi test test-unit test-python test-integration test-robot test-all build-test-artifacts build-tools install-hooks sync-versions check-versions docker-dev docker-all docker-base docker-toolchain docker-devenv docker-builder docker-runtime ci-local ci-full perf-bench perf-check perf-baseline

# By default, perform an incremental build
all: build

# ------------------------------------------------------------------------------
# FFI Layout Verification
# ------------------------------------------------------------------------------

# Verify that Rust struct layouts match the QEMU binary ground truth.
check-ffi:
	@echo "==> Verifying FFI layouts..."
	@./scripts/check-ffi.py

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
	uv sync --active --link-mode=copy
	@echo "✓ Virtual environment synchronized with uv."
	@echo "✓ Activate with: source .venv/bin/activate"

# Run integration smoke tests (Bash/QEMU level tests for phases 1 & 2)
test-integration: venv
	uv run --active $(MAKE) build-test-artifacts
	@bash scripts/cleanup-sim.sh --quiet
	@echo "==> Running Modernized Integration Tests (via pytest)..."
	uv run --active pytest tests/test_phase1.py tests/test_phase2.py tests/test_phase3.py \
		tools/testing/test_qmp.py tests/test_qmp_bridge.py tests/test_qemu_library_pytest.py \
		tests/test_phase6.py tests/test_phase7.py \
		tests/test_phase8.py tests/test_phase10.py tests/test_phase12.py \
		-v -n auto --tb=short
	@echo "==> Running Legacy Integration Tests (Bash scripts)..."

	@for test_script in test/phase11_3/smoke_test.sh test/phase11/smoke_test.sh \
		test/phase13/smoke_test.sh test/phase14/smoke_test.sh test/phase15/smoke_test.sh \
		test/phase16/smoke_test.sh test/phase3.5/smoke_test.sh test/phase5/smoke_test.sh \
		test/phase9/smoke_test.sh test/actuator/smoke_test.sh; do \
		echo "--> Running $$test_script"; \
		uv run --active bash "$$test_script" || { bash scripts/cleanup-sim.sh; exit 1; }; \
		bash scripts/cleanup-sim.sh --quiet; \
	done
	@echo "✓ All integration tests passed."

# Run integration tests compiled with C/C++ memory sanitizers (ASan/UBSan)
test-asan: venv
	uv run --active $(MAKE) build-test-artifacts
	@echo "==> Building QEMU with ASan/UBSan enabled..."
	VIRTMCU_USE_ASAN=1 bash scripts/setup-qemu.sh --force
	@bash scripts/cleanup-sim.sh --quiet
	@echo "==> Running integration tests under ASan/UBSan..."
	ASAN_OPTIONS=detect_leaks=1,halt_on_error=1,detect_stack_use_after_return=1 \
	UBSAN_OPTIONS=halt_on_error=1:print_stacktrace=1 \
	$(MAKE) test-integration
	@echo "✓ All ASan integration tests passed."

# Run Miri to detect Undefined Behavior in pure-Rust logic and safe FFI wrappers
test-miri:
	@echo "==> Running cargo miri test..."
	@if ! rustup component list --toolchain nightly 2>/dev/null | grep -q "miri.*(installed)"; then \
		echo "Installing Rust nightly toolchain and Miri..."; \
		rustup toolchain install nightly --profile minimal --component miri; \
	fi
	@cargo +nightly miri setup
	@MIRIFLAGS="-Zmiri-disable-isolation" cargo +nightly miri test
	@echo "✓ Miri tests passed."


# Run all Python unit tests (no QEMU required).
test-unit: venv
	@echo "==> Running Tier 1 Unit Tests (no QEMU)..."
	@./scripts/cleanup-sim.sh --quiet
	PYTHONPATH=$(CURDIR) uv run --active pytest \
		tests/repl2qemu/ tests/test_yaml2qemu.py tests/test_cli_generator.py \
		tests/test_fdt_emitter.py tests/test_qmp_bridge.py tests/test_vproto.py \
		tests/test_telemetry_listener.py tests/test_telemetry_fbs.py tests/test_fake_adapter.py \
		tests/test_mcp_server/ \
		-v -n auto --tb=short

test: test-unit

# Run Robot Framework integration tests (requires QEMU built via make setup-initial).
test-robot: venv
	export PYTHONPATH=$(CURDIR) && \
	uv run --active robot \
	  --outputdir test-results/robot \
	  --loglevel INFO \
	  tests/test_qmp_keywords.robot \
	  tests/test_interactive_echo.robot

# Run guest firmware coverage analysis (Phase 1)
test-coverage-guest:
	@echo "==> Running guest firmware coverage (drcov) inside builder..."
	@bash scripts/docker-build.sh builder
	@docker run --rm \
		-v "$(CURDIR):/workspace" -w /workspace \
		-e PYTHONPATH=/workspace \
		$(BUILDER_IMG) \
		bash -c "make -C test/phase1 && \
			 DRCOV_SO=\$$(find /opt/virtmcu/lib/qemu/plugins /build/qemu -name 'libdrcov.so' 2>/dev/null | head -n 1) && \
			 qemu-system-arm -M arm-generic-fdt,hw-dtb=test/phase1/minimal.dtb \
			   -kernel test/phase1/hello.elf -nographic -m 128M -display none \
			   -plugin \"\$$DRCOV_SO\",filename=hello.drcov -d plugin & \
			 sleep 2 && kill -INT \$$! && wait \$$! || true; \
			 python3 tools/analyze_coverage.py hello.drcov test/phase1/hello.elf --fail-under 80"
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
		--directory $(QEMU_BUILD)/libhw-virtmcu-zenoh-clock.a.p \
		--directory $(QEMU_BUILD)/libhw-virtmcu-zenoh-chardev.a.p \
		--directory $(QEMU_BUILD)/libhw-virtmcu-zenoh-netdev.a.p \
		--directory $(QEMU_BUILD)/libhw-virtmcu-zenoh-actuator.a.p \
		--directory $(QEMU_BUILD)/libhw-virtmcu-zenoh-telemetry.a.p \
		--directory $(QEMU_BUILD)/libhw-virtmcu-zenoh-802154.a.p \
		--directory $(QEMU_BUILD)/libhw-virtmcu-zenoh-ui.a.p \
		--directory $(QEMU_BUILD)/libhw-virtmcu-test-qom-device.a.p \
		--output-file test-results/coverage/host.info --rc branch_coverage=1 --ignore-errors empty
	lcov --quiet --extract test-results/coverage/host.info "*/hw/virtmcu/*" --output-file test-results/coverage/host_filtered.info --rc branch_coverage=1
	genhtml --quiet test-results/coverage/host_filtered.info --output-directory test-results/coverage/html --title "virtmcu Host Coverage" --legend --branch-coverage
	@echo "✓ Report generated: test-results/coverage/html/index.html"

build-test-artifacts:
	@$(MAKE) -C test/phase1 -j$(JOBS)
	@$(MAKE) -C test/phase8 -j$(JOBS)
	@$(MAKE) -C test/phase12 -j$(JOBS)
	@$(MAKE) -C test/actuator -j$(JOBS)
	@$(MAKE) -C test/riscv -j$(JOBS)
	@$(MAKE) -C test/phase27 -j$(JOBS)
	@echo "==> Building test tools (zenoh_coordinator, cyber_bridge)..."
	@cargo build --release -p zenoh_coordinator -p cyber_bridge

# Run the complete test suite: unit tests, integration smoke tests, Robot tests.
test-all: test test-integration test-robot test-coverage-guest

# Run integration smoke tests inside the builder container (Safe for macOS hosts).
test-integration-docker:
	@echo "==> Running integration tests inside builder container..."
	@bash scripts/docker-build.sh builder
	@mkdir -p $(CARGO_CACHE_DIR)
	docker run --rm \
		--user $$(id -u):$$(id -g) \
		-e HOME=/tmp \
		-e USER=vscode \
		-e CARGO_HOME=/workspace/.cargo-cache \
		-e UV_PROJECT_ENVIRONMENT=/workspace/.venv-docker \
		-v "$(CURDIR):/workspace" -w /workspace \
		-e PYTHONPATH=/workspace \
		-e VIRTMUC_SKIP_QEMU_HEADERS_WARNING=1 \
		-e VIRTMCU_SKIP_BUILD_DIR=1 \
		$(BUILDER_IMG) \
		bash -c "make test-integration"
	@echo "✓ All integration tests passed inside container."
# ------------------------------------------------------------------------------
# Lint & Format
# ------------------------------------------------------------------------------

lint: venv check-versions check-ffi lint-python lint-python-types lint-rust lint-c lint-shell lint-docker lint-yaml lint-actions lint-meson lint-spelling lint-audit lint-docs
	@echo "All linting and static analysis checks passed!"

lint-docs:
	@echo "==> Checking Rust documentation..."
	@RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
	@echo "✓ Rust documentation check passed."

lint-audit:
	@echo "==> Security Audit (Rust)..."
	@if command -v cargo-audit >/dev/null 2>&1; then \
		cargo audit --ignore RUSTSEC-2026-0041 --ignore RUSTSEC-2023-0071 --ignore RUSTSEC-2024-0436 --ignore RUSTSEC-2025-0134 -f Cargo.lock; \
	else \
		echo "⚠️  cargo-audit not installed. Skipping Rust security audit. (Run 'cargo install cargo-audit' to enable)"; \
	fi

	@echo "==> cargo deny (supply chain security)..."
	@if command -v cargo-deny >/dev/null 2>&1; then \
		cargo deny check && echo "✓ cargo deny passed." || { echo "❌ cargo deny failed"; exit 1; }; \
	else \
		echo "⚠️  cargo-deny not installed. Skipping Rust supply chain audit. (Run 'cargo install cargo-deny' to enable)"; \
	fi
	@echo "✓ Audit checks completed."
lint-python:
	@echo "==> ruff check..."
	uv run --active ruff check .
	@echo "✓ ruff passed."

lint-spelling:
	@echo "==> codespell..."
	@uvx codespell --skip="./third_party/*,./.venv/*,**/build/*,**/target/*,./.git/*,./.claude/*,Cargo.lock,uv.lock,./patches/*,./coverage_report/*,./test-results/*,./.cargo-cache/*" \
		--ignore-words-list="virtmcu,zenoh,qemu,qmp,riscv,TE" .
	@echo "✓ codespell passed."


lint-shell:
	@echo "==> shellcheck..."
	@shellcheck --version >/dev/null 2>&1 || { echo "❌ Error: shellcheck is not installed. Install with: sudo apt-get install shellcheck"; exit 1; }
	@find . -type f -name "*.sh" -not -path "*/third_party/*" -not -path "*/.venv/*" -not -path "*/build/*" -not -path "*/.cargo-cache/*" -print0 | xargs -0 shellcheck --severity=warning
	@echo "✓ shellcheck passed."


lint-docker:
	@echo "==> hadolint..."
	@hadolint --version >/dev/null 2>&1 || { echo "❌ Error: hadolint is not installed. Install from: https://github.com/hadolint/hadolint"; exit 1; }
	@hadolint docker/Dockerfile
	@echo "✓ hadolint passed."

lint-actions:
	@echo "==> actionlint..."
	@actionlint -version >/dev/null 2>&1 || { echo "❌ Error: actionlint is not installed. Install with: (curl -s https://raw.githubusercontent.com/rhysd/actionlint/main/scripts/download-actionlint.bash | bash) && sudo mv actionlint /usr/local/bin/"; exit 1; }
	@actionlint
	@echo "✓ actionlint passed."


lint-yaml:
	@echo "==> yamllint..."
	@uvx yamllint -d "{extends: relaxed, rules: {line-length: disable}}" $$(find . -type f \( -name "*.yml" -o -name "*.yaml" \) -not -path "*/third_party/*" -not -path "*/.venv/*" -not -path "*/build/*" -not -path "*/target/*" -not -path "*/.claude/*" -not -path "*/.cargo-cache/*")
	@echo "✓ yamllint passed."

lint-python-types:
	@echo "==> mypy..."
	@uv run --active mypy tools/ tests/ patches/
	@echo "✓ mypy passed."
lint-c:
	@echo "==> clang-format (dry-run)..."
	@clang-format --version >/dev/null 2>&1 || { echo "❌ Error: clang-format is not installed. Install with: sudo apt-get install clang-format"; exit 1; }
	@find hw tools test -type f \( -name "*.c" -o -name "*.h" -o -name "*.cpp" -o -name "*.cc" -o -name "*.hpp" \) \
		-not -path "*/rust/*" \
		-not -path "*/remote-port/*" \
		-not -path "*/third_party/*" \
		-not -path "*/build/*" \
		-not -path "*/.venv/*" \
		-not -path "*/target/*" \
		-print0 | xargs -0 clang-format -Werror --dry-run
	@echo "✓ clang-format passed."
	@echo "==> cppcheck..."
	@cppcheck --version >/dev/null 2>&1 || { echo "❌ Error: cppcheck is not installed. Install with: sudo apt-get install cppcheck"; exit 1; }
	@cppcheck --error-exitcode=1 --enable=warning,style,performance,portability --quiet --std=c11 \
		--suppress=unusedFunction \
		--suppress=arithOperationsOnVoidPointer \
		--suppress=normalCheckLevelMaxBranches \
		--suppress=uninitvar --suppress=legacyUninitvar \
		--suppress=knownConditionTrueFalse \
		--suppress=identicalInnerCondition \
		--suppress=dangerousTypeCast \
		--suppress=constVariablePointer --suppress=constParameterPointer \
		--suppress=redundantAssignment --suppress=noExplicitConstructor \
		--suppress=constParameterCallback --suppress=unusedVariable \
		-i tools/systemc_adapter/build/ \
		-DDEFINE_TYPES= -DHWADDR_PRIx=\"lx\" -DPRIx64=\"llx\" \
		hw/misc/ tools/systemc_adapter/
	@echo "✓ cppcheck passed."


lint-rust:
	@echo "==> Checking Cargo workspace version synchronization..."
	@cargo metadata --no-deps --format-version 1 | \
		python3 -c "import sys,json; m=json.load(sys.stdin); vs=set(p['version'] for p in m['packages']); assert len(vs)==1, f'version drift: {vs}'"
	@echo "✓ Cargo workspace versions aligned."
	@echo "==> Running cargo fmt --check..."
	@cargo fmt --all --check
	@echo "==> Running cargo machete..."
	@cargo machete
	@echo "==> Running cargo clippy..."
	@cargo clippy --workspace

lint-meson:
	@echo "==> Running meson format..."
	@uvx meson format -q hw/meson.build
	@echo "✓ meson format passed."
build-tools:
	@echo "==> Building virtmcu-tools package..."
	@cd packaging/virtmcu-tools && uv build >/dev/null && \
		WHEEL_FILE=$$(ls dist/*.whl | head -n 1) && \
		unzip -l "$$WHEEL_FILE" | grep "virtmcu_tools/repl2qemu/" >/dev/null && \
		unzip -l "$$WHEEL_FILE" | grep "virtmcu_tools/yaml2qemu.py" >/dev/null && \
		unzip -l "$$WHEEL_FILE" | grep "virtmcu_tools/mcp_server/" >/dev/null && \
		unzip -l "$$WHEEL_FILE" | grep "virtmcu_tools/qmp_bridge.py" >/dev/null && \
		echo "✓ virtmcu-tools package build passed."

fmt: fmt-python fmt-rust fmt-meson fmt-c fmt-yaml

fmt-python: venv
	@echo "==> ruff format + fix..."
	uv run --active ruff format .
	uv run --active ruff check . --fix

fmt-rust:
	@echo "==> cargo fmt..."
	@cargo fmt --all

fmt-meson:
	@echo "==> meson format..."
	@meson fmt -i hw/meson.build && echo "✓ meson format passed." || { echo "❌ meson format failed"; exit 1; }

fmt-c:
	@echo "==> clang-format..."
	@find hw -type f \( -name "*.c" -o -name "*.h" \) -not -path "*/rust/*" -not -path "*/third_party/*" -print0 | xargs -0 clang-format -i && echo "✓ clang-format passed." || { echo "❌ clang-format failed"; exit 1; }

fmt-yaml:
	@echo "==> stripping trailing whitespace from YAMLs..."
	@find . -type f \( -name "*.yml" -o -name "*.yaml" \) -not -path "*/third_party/*" -not -path "*/.venv/*" -print0 | xargs -0 sed -i 's/[[:space:]]*$$//'

install-hooks:
	@echo "==> Installing Git hooks..."
	@mkdir -p .git/hooks
	@printf '#!/bin/sh\nset -e\nmake ci-local\n' > .git/hooks/pre-commit
	@printf '#!/bin/sh\nset -e\nmake test-integration-docker\n' > .git/hooks/pre-push
	@chmod +x .git/hooks/pre-push .git/hooks/pre-commit
	@echo "✓ hooks installed: pre-commit (ci-local), pre-push (test-integration-docker)."

# ------------------------------------------------------------------------------
# Performance Benchmarking & Trend Tracking (Phase 16)
# ------------------------------------------------------------------------------

# Run the full performance benchmark and save results to test/phase16/last_results.json.
perf-bench: venv
	@$(MAKE) -C test/phase16 bench.elf
	PYTHONPATH=$(CURDIR) uv run --active python3 test/phase16/bench.py

# Save the current benchmark results as the performance baseline.
perf-baseline: perf-bench
	uv run --active python3 scripts/perf_trend.py --save-baseline
	@echo "✓ Performance baseline updated."

# Check current benchmark results against the saved baseline; exit 1 on regression.
perf-check: venv
	@if [ ! -f test/phase16/last_results.json ]; then \
		$(MAKE) -C test/phase16 bench.elf && PYTHONPATH=$(CURDIR) uv run --active python3 test/phase16/bench.py; \
	fi
	uv run --active python3 scripts/perf_trend.py --check

# ------------------------------------------------------------------------------
# Local CI Simulation
# Mirrors the GitHub CI tier structure so you can validate locally before push.
#
#   make ci-local   — Fast path for git commit hooks. Runs lints and unit tests
#                    inside the devenv-base container. Does NOT build the container.
#                    (Run 'make docker-dev' manually if the image is missing or stale).
#
#   make ci-full    — ci-local + ci-asan + ci-miri + builder stage (QEMU compile) +
#                    runtime image + representative phase smoke tests inside container.
#                    This is the closest local equivalent to the full GitHub CI run.
#
# All linting tools are mandatory for 1:1 parity with GitHub CI.
# ------------------------------------------------------------------------------

ci-local:
	@echo "════════════════════════════════════════════════════"
	@echo "  CI Tier 1 — Running Lints & Unit Tests inside container"
	@echo "════════════════════════════════════════════════════"
	@bash scripts/docker-build.sh devenv-base
	@echo ""
	# Run lints and unit tests strictly inside the devenv-base container.
	# We map the current host user's UID/GID into the container to ensure that 
	# any generated files (like .venv) are owned by YOU, not root.
	@mkdir -p $(CARGO_CACHE_DIR)
	docker run --rm \
		--user $$(id -u):$$(id -g) \
		-e HOME=/tmp \
		-e USER=vscode \
		-e CARGO_HOME=/workspace/.cargo-cache \
		-e VIRTMUC_SKIP_QEMU_HEADERS_WARNING=1 \
		-e UV_PROJECT_ENVIRONMENT=/workspace/.venv-docker \
		-v "$(CURDIR):/workspace" -w /workspace \
		$(DEVENV_BASE_IMG) bash -c "make lint && make test-unit"
	@echo ""
	@echo "✓ ci-local passed (parity with GitHub CI Tier 1)."
	@echo "  To run the full pipeline (builder ~40 min + integration tests): make ci-full"

# Run host-side C coverage for peripheral plugins (inside builder)
test-coverage-peripheral:
	@echo "==> Running peripheral C coverage (gcovr)..."
	@bash scripts/docker-build.sh builder
	@mkdir -p test-results
	@docker run --rm \
		-v "$(CURDIR):/workspace" -w /workspace \
		$(BUILDER_IMG) \
		bash -c "gcovr -r /build/qemu/hw/virtmcu \
			--gcov-executable gcov \
			--object-directory /build/qemu/build-virtmcu \
			--object-directory /workspace/coverage-data \
			--xml /workspace/test-results/peripheral-coverage.xml \
			--html-details /workspace/test-results/peripheral-coverage.html \
			--print-summary"

ci-full: ci-local ci-asan ci-miri
	@echo ""
	@echo "════════════════════════════════════════════════════"
	@echo "  CI Full — Docker: builder + runtime"
	@echo "════════════════════════════════════════════════════"
	@bash scripts/docker-build.sh builder
	@bash scripts/docker-build.sh runtime
	@echo ""
	@echo "════════════════════════════════════════════════════"
	@echo "  CI Full — Integration smoke tests matrix (inside builder)"
	@echo "════════════════════════════════════════════════════"
	@mkdir -p coverage-data
	@echo "==> Running full smoke test matrix (Equivalent to GitHub CI)..."
	docker run --rm \
		-v "$(CURDIR):/workspace" -w /workspace \
		-e PYTHONPATH=/workspace \
		-e VIRTMCU_STALL_TIMEOUT_MS=120000 \
		-e GCOV_PREFIX=/workspace/coverage-data \
		-e GCOV_PREFIX_STRIP=3 \
		$(BUILDER_IMG) \
		bash scripts/ci-phase.sh all
	@echo ""
	@echo "════════════════════════════════════════════════════"
	@echo "  CI Full — Coverage Checks"
	@echo "════════════════════════════════════════════════════"
	$(MAKE) test-coverage-guest
	$(MAKE) test-coverage-peripheral
	@echo ""
	@echo "✓ ci-full passed."

ci-asan:
	@echo "════════════════════════════════════════════════════"
	@echo "  CI ASan — Docker: devenv-base"
	@echo "════════════════════════════════════════════════════"
	@bash scripts/docker-build.sh devenv-base
	@echo ""
	@echo "════════════════════════════════════════════════════"
	@echo "  CI ASan — Building and testing under ASan"
	@echo "════════════════════════════════════════════════════"
	@mkdir -p $(CARGO_CACHE_DIR)
	docker run --rm \
		--user $$(id -u):$$(id -g) \
		-e HOME=/tmp \
		-e USER=vscode \
		-e CARGO_HOME=/workspace/.cargo-cache \
		-e VIRTMUC_SKIP_QEMU_HEADERS_WARNING=1 \
		-e UV_PROJECT_ENVIRONMENT=/workspace/.venv-docker \
		-e VIRTMCU_STALL_TIMEOUT_MS=300000 \
		-v "$(CURDIR):/workspace" -w /workspace \
		$(DEVENV_BASE_IMG) make test-asan
	@echo ""
	@echo "✓ ci-asan passed."

ci-miri:
	@echo "════════════════════════════════════════════════════"
	@echo "  CI Miri — Docker: devenv-base"
	@echo "════════════════════════════════════════════════════"
	@bash scripts/docker-build.sh devenv-base
	@echo ""
	@echo "════════════════════════════════════════════════════"
	@echo "  CI Miri — Running Miri tests"
	@echo "════════════════════════════════════════════════════"
	docker run --rm \
		-v "$(CURDIR):/workspace" \
		-v /workspace/third_party \
		-w /workspace \
		$(DEVENV_BASE_IMG) make test-miri
	@echo ""
	@echo "✓ ci-miri passed."

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