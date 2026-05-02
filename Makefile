ARCH ?= $(shell uname -m | sed -e "s/x86_64/amd64/" -e "s/aarch64/arm64/")
IMAGE_TAG ?= dev
DEVENV_BASE_IMG ?= ghcr.io/refractsystems/virtmcu/devenv-base:$(IMAGE_TAG)-$(ARCH)
BUILDER_IMG ?= ghcr.io/refractsystems/virtmcu/builder:$(IMAGE_TAG)-$(ARCH)
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
#   make run     — Launch QEMU using the minimal boot_arm test DTB.
#
# Environment Variables / Flags:
#   VIRTMCU_SKIP_BUILD_DIR=1  — Forces `scripts/run.sh` to bypass the local build 
#			       directory (`third_party/qemu/build-virtmcu`) and 
#			       strictly use installed artifacts in `/opt/virtmcu`. 
#			       Essential for CI and Docker targets testing final images.
#   VIRTMCU_STALL_TIMEOUT_MS  — Milliseconds the Python orchestrator and clock 
#			       will wait for a QEMU TCG quantum to complete before 
#			       declaring a clock stall. Increased for ASan (e.g., 300000ms).
#   VIRTMCU_USE_ASAN=1	— Compiles QEMU and Rust plugins with Memory Sanitizer 
#			       (ASan) and UndefinedBehaviorSanitizer (UBSan) enabled. 
#			       Output goes to `build-virtmcu-asan`.
#   PYTEST_WORKERS=N	  — Number of parallel workers for `pytest -n`. Defaults to `auto`.
#
# Advanced CI/Testing Flags:
#   ASAN_OPTIONS	      — Runtime options for AddressSanitizer (e.g., detect_leaks=0).
#   UBSAN_OPTIONS	     — Runtime options for UndefinedBehaviorSanitizer.
#   MIRIFLAGS		 — Flags passed to cargo miri test (e.g., -Zmiri-disable-isolation).
#   VIRTMCU_SKIP_QEMU_HEADERS_WARNING=1 — Silences warning about missing QEMU headers in local-ci.
#   UV_PROJECT_ENVIRONMENT    — Path to the isolated virtual environment for Docker test runs.
#   GCOV_PREFIX / GCOV_PREFIX_STRIP — Used to correctly map host-side C coverage paths inside Docker.
# ==============================================================================

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
# Automatically determine the number of parallel jobs for make
ifeq ($(CI),true)
  JOBS ?= 1
else
  JOBS ?= $(shell nproc 2>/dev/null || sysctl -n hw.logicalcpu 2>/dev/null || echo 4)
endif

.PHONY: all setup-initial build run clean clean-sim clean-debug distclean venv fmt fmt-python fmt-rust fmt-c fmt-meson fmt-yaml lint lint-python lint-simulation-usage lint-python-types lint-rust lint-c lint-shell lint-docker lint-yaml lint-actions lint-meson lint-spelling check-ffi build-test-artifacts build-tools install-hooks sync-versions check-versions docker-dev docker-all docker-base docker-toolchain docker-devenv docker-builder docker-runtime ci-local ci-smoke ci-full perf-bench perf-check perf-baseline tag

# By default, perform an incremental build
all: build

# ------------------------------------------------------------------------------
# FFI Layout Verification
# ------------------------------------------------------------------------------

# Verify that Rust struct layouts match the QEMU binary ground truth.
check-ffi:
	@echo "==> Verifying FFI layouts..."
	@uv run --active python3 scripts/check-ffi.py

# ------------------------------------------------------------------------------
# Version Management
# ------------------------------------------------------------------------------

# Propagate versions from the BUILD_DEPS file to all downstream configuration files.
sync-versions:
	@echo "==> Synchronizing dependency versions..."
	@uv run --active python3 scripts/sync-versions.py
	@echo "✓ Versions synchronized."

# Verify that all versions are in sync across the codebase.
check-versions:
	@echo "==> Checking version synchronization..."
	@uv run --active python3 scripts/check-versions.py

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
	  $(if $(wildcard tests/fixtures/guest_apps/boot_arm/minimal.dtb),--dtb tests/fixtures/guest_apps/boot_arm/minimal.dtb) \
	  $(if $(wildcard tests/fixtures/guest_apps/boot_arm/hello.elf),--kernel tests/fixtures/guest_apps/boot_arm/hello.elf) \
	  -nographic \
	  -m 128M \
	  $(EXTRA_ARGS)

# Launch the emulator using strictly the installed binaries in /opt/virtmcu 
# (ignores local build directory).
run-installed:
	@VIRTMCU_SKIP_BUILD_DIR=1 bash scripts/run.sh \
	  $(if $(wildcard tests/fixtures/guest_apps/boot_arm/minimal.dtb),--dtb tests/fixtures/guest_apps/boot_arm/minimal.dtb) \
	  $(if $(wildcard tests/fixtures/guest_apps/boot_arm/hello.elf),--kernel tests/fixtures/guest_apps/boot_arm/hello.elf) \
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

# Run integration smoke tests (Bash/QEMU level tests for boot_arm and other domains)
test-integration: venv
	uv run --active $(MAKE) build-test-artifacts
	@bash scripts/cleanup-sim.sh --quiet
	@echo "==> Running Modernized Integration Tests (via pytest)..."
	uv run --active pytest tests/integration/ \
		-v -n $(PYTEST_WORKERS) --tb=short --capture=sys
	@echo "==> Running Legacy Integration Tests (Bash scripts)..."

	@for test_script in tests/fixtures/guest_apps/riscv_complex/smoke_test.sh \
		tests/fixtures/guest_apps/priority_routing/smoke_test.sh tests/fixtures/guest_apps/complex_board/smoke_test.sh tests/fixtures/guest_apps/coverage_gap/smoke_test.sh \
		tests/fixtures/guest_apps/perf_bench/smoke_test.sh tests/fixtures/guest_apps/yaml_boot_advanced/smoke_test.sh tests/fixtures/guest_apps/irq_stress/smoke_test.sh \
		tests/fixtures/guest_apps/ftrt_timing/smoke_test.sh; do \
		echo "--> Running $$test_script"; \
		uv run --active bash "$$test_script" || { bash scripts/cleanup-sim.sh; exit 1; }; \
		bash scripts/cleanup-sim.sh --quiet; \
	done
	@echo "✓ All integration tests passed."

# Run integration tests compiled with C/C++ memory sanitizers (ASan/UBSan)
test-asan: venv
	@echo "==> Building QEMU with ASan/UBSan enabled..."
	VIRTMCU_USE_ASAN=1 bash scripts/setup-qemu.sh --force
	@bash scripts/cleanup-sim.sh --quiet
	@echo "==> Running integration tests under ASan/UBSan..."
	VIRTMCU_USE_ASAN=1 \
	VIRTMCU_STALL_TIMEOUT_MS=300000 \
	ASAN_OPTIONS=detect_leaks=0,halt_on_error=1,detect_stack_use_after_return=1 \
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


# Parallelism for pytest (default to auto, can be overridden in CI or hooks)
PYTEST_WORKERS ?= auto

test-plugins-load: build
	@echo "==> Running plugin smoke test..."
	@uv run --active python3 scripts/test-plugins-load.py

# Run all Python unit tests (no QEMU required).
test-unit: venv
	@echo "==> Running Tier 1 Unit Tests (no QEMU)..."
	@./scripts/cleanup-sim.sh --quiet
	PYTHONPATH=$(CURDIR) uv run --active pytest \
		tests/unit/ \
		-v -n $(PYTEST_WORKERS) --tb=short --capture=sys

# Alias for test-unit
test: test-unit

# Run Robot Framework integration tests (requires QEMU built via make setup-initial).
test-robot: venv
	export PYTHONPATH=$(CURDIR) && \
	uv run --active robot \
	  --outputdir test-results/robot \
	  --loglevel INFO \
	  tests/integration/peripherals/test_uart_interactive.robot

# Run guest firmware coverage analysis (boot_arm)
test-coverage-guest:
	@echo "==> Running guest firmware coverage (drcov) inside builder..."
	@bash scripts/docker-build.sh builder
	@docker run --rm \
		-v "$(CURDIR):/workspace" -w /workspace \
		-e PYTHONPATH=/workspace \
		-e CI=true \
		$(BUILDER_IMG) \
		bash -c "make -C tests/fixtures/guest_apps/boot_arm && \
			 DRCOV_SO=\$$(find /opt/virtmcu/lib/qemu/plugins /build/qemu -name 'libdrcov.so' 2>/dev/null | head -n 1) && \
			 qemu-system-arm -M arm-generic-fdt,hw-dtb=tests/fixtures/guest_apps/boot_arm/minimal.dtb \
			   -kernel tests/fixtures/guest_apps/boot_arm/hello.elf -nographic -m 128M -display none \
			   -plugin \"\$$DRCOV_SO\",filename=hello.drcov -d plugin & \
			 sleep 2 && kill -INT \$$! && wait \$$! || true; \
			 python3 tools/analyze_coverage.py hello.drcov tests/fixtures/guest_apps/boot_arm/hello.elf --fail-under 80"
	@echo "✓ Guest coverage check passed."

# Generate host-side C/Rust coverage report (requires lcov)
coverage-report:
	@echo "==> Generating host-side coverage report..."
	@mkdir -p test-results/coverage
	@# Search for .gcda files in both the build directory and the isolated coverage data directory
	lcov --quiet --capture \
		--directory $(QEMU_BUILD) \
		--directory $(CURDIR)/target/coverage \
		--output-file test-results/coverage/host.info --rc branch_coverage=1 --ignore-errors empty
	lcov --quiet --extract test-results/coverage/host.info "*/hw/virtmcu/*" --output-file test-results/coverage/host_filtered.info --rc branch_coverage=1
	genhtml --quiet test-results/coverage/host_filtered.info --output-directory test-results/coverage/html --title "virtmcu Host Coverage" --legend --branch-coverage
	@echo "✓ Report generated: test-results/coverage/html/index.html"

# Builds all test artifacts across all domains
build-test-artifacts:
	@$(MAKE) -C tests/fixtures/guest_apps/boot_arm -j$(JOBS)
	@$(MAKE) -C tests/fixtures/guest_apps/uart_echo -j$(JOBS)
	@$(MAKE) -C tests/fixtures/guest_apps/telemetry_wfi -j$(JOBS)
	@$(MAKE) -C tests/fixtures/guest_apps/actuator -j$(JOBS)
	@$(MAKE) -C tests/fixtures/guest_apps/boot_riscv -j$(JOBS)
	@$(MAKE) -C tests/fixtures/guest_apps/flexray_bridge -j$(JOBS)
	@if [ "$$CI" = "true" ] && command -v deterministic_coordinator >/dev/null 2>&1; then \
		echo "==> CI detected: Skipping Rust tools build (using pre-compiled binary in PATH)"; \
	else \
		echo "==> Building test tools (deterministic_coordinator, cyber_bridge)..."; \
		cargo build --release -j$(JOBS) -p deterministic_coordinator -p cyber_bridge; \
	fi

# Run the complete test suite: unit tests, integration smoke tests, Robot tests.
test-all: test test-integration test-robot test-coverage-guest

# Run integration smoke tests inside the builder container (Safe for macOS hosts).
test-integration-docker:
	@echo "==> Running integration tests inside builder container..."
	@bash scripts/docker-build.sh builder
	docker run --rm \
		--user $$(id -u):$$(id -g) \
		-e HOME=/tmp \
		-e USER=vscode \
		-e CI=true \
		-e CARGO_TARGET_DIR=/tmp/ci-target \
		-e UV_PROJECT_ENVIRONMENT=/workspace/.venv-docker \
		-v "$(CURDIR):/workspace" -w /workspace \
		-v ci-cargo-registry:/usr/local/cargo/registry \
		-e PYTHONPATH=/workspace \
		-e VIRTMUC_SKIP_QEMU_HEADERS_WARNING=1 \
		-e VIRTMCU_SKIP_BUILD_DIR=1 \
		$(BUILDER_IMG) \
		bash -c "make test-integration"
	@echo "✓ All integration tests passed inside container."
# ------------------------------------------------------------------------------
# Lint & Format
# ------------------------------------------------------------------------------

# Run all linting and static analysis checks
lint: venv check-versions check-ffi lint-exports lint-python lint-python-types lint-rust lint-c lint-shell lint-docker lint-yaml lint-actions lint-meson lint-spelling lint-audit lint-docs
	@echo "All linting and static analysis checks passed!"

# Verify that plugins export required unmangled FFI symbols
lint-exports:
	@echo "==> Verifying plugin exports..."
	@uv run --active python3 scripts/verify-exports.py
	@echo "✓ Plugin exports verified."

# Check Rust documentation for warnings
lint-docs:
	@echo "==> Checking Rust documentation..."
	@RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
	@echo "✓ Rust documentation check passed."

# Run Rust security audit and supply chain checks
lint-audit:
	@echo "==> Security Audit (Rust)..."
	@if command -v cargo-audit >/dev/null 2>&1; then \
		cargo audit --ignore RUSTSEC-2026-0041 --ignore RUSTSEC-2023-0071 --ignore RUSTSEC-2024-0436 --ignore RUSTSEC-2025-0134 -f Cargo.lock; \
	else \
		echo "❌ cargo-audit not installed. Run 'cargo install cargo-audit' to enable."; exit 1; \
	fi

	@echo "==> cargo deny (supply chain security)..."
	@if command -v cargo-deny >/dev/null 2>&1; then \
		cargo deny check && echo "✓ cargo deny passed." || { echo "❌ cargo deny failed"; exit 1; }; \
	else \
		echo "❌ cargo-deny not installed. Run 'cargo install cargo-deny' to enable."; exit 1; \
	fi
	@echo "✓ Audit checks completed."
# Run Python linting and type checking
lint-python:
	@echo "==> Check for banned struct usage..."
	@if grep -rnE "struct\.(pack|unpack|Struct)|import struct|from struct|Struct\(" tests/ tools/ docs/tutorials/ | grep -vE "proto_gen.py|vproto\.py|tools/README\.md" ; then \
		echo "❌ ERROR: Banned struct usage detected. Use vproto.py, FlatBuffers, or int.from_bytes/to_bytes instead."; exit 1; \
	fi
	@echo "==> Check for banned struct in scripts (limited)..."
	@if grep -rnE "struct\.(pack|unpack)" scripts/ ; then \
		echo "❌ ERROR: Banned struct.pack/unpack in scripts."; exit 1; \
	fi
	@echo "==> Check for hardcoded stall-timeout..."
	@if grep -rE "stall-timeout=[0-9]+" tests/ tools/ ; then \
		echo "❌ ERROR: Hardcoded stall-timeout detected. Use dynamic scaling via VIRTMCU_STALL_TIMEOUT_MS."; exit 1; \
	fi
	@echo "==> Check for banned sleep calls (asyncio.sleep / time.sleep)..."
	@violations=$$(grep -rnE "(asyncio|time)\.sleep\(" tests/ tools/ docs/tutorials/ | grep -v "SLEEP_EXCEPTION:" || true); \
	if [ -n "$$violations" ]; then \
	        echo "❌ ERROR: Banned sleep call found in tests/tools/tutorials (use vta.step or transport signaling instead):"; \
	        echo "$$violations"; \
	        exit 1; \
	fi
	@echo "==> Check for raw zenoh.open() in pytest scope (must use make_client_config / zenoh_session fixture)..."
	@# Default zenoh.Config() opens in peer mode with multicast scouting enabled,
	@# causing parallel pytest workers to silently discover each other across the
	@# container's network namespace and cross-talk on shared topics. CLAUDE.md
	@# Second Priority / ADR-014 BANS runtime peer-mode scouting.
	@# All Zenoh sessions in pytest-collected tests and shared testing infrastructure
	@# MUST use make_client_config() (or the zenoh_session fixture, which wraps it).
	@# Approved exceptions must carry an inline # ZENOH_OPEN_EXCEPTION: <reason> comment.
	@# Scope: tests/integration/, tests/unit/, tests/system/, tests/*.py, tools/testing/.
	@# Fixture scripts under tests/fixtures/guest_apps/ are standalone (not pytest-parallel) and exempt.
	@violations=$$(grep -rnE "zenoh\.open\(" tests/integration/ tests/unit/ tests/system/ tools/testing/ --include="*.py" 2>/dev/null \
		| grep -v "# ZENOH_OPEN_EXCEPTION:" || true); \
	tests_root=$$(grep -nE "zenoh\.open\(" tests/*.py 2>/dev/null | grep -v "# ZENOH_OPEN_EXCEPTION:" || true); \
	violations="$$violations$$tests_root"; \
	if [ -n "$$violations" ]; then \
		echo "❌ ERROR: Raw zenoh.open() found in pytest scope. Use make_client_config() / zenoh_session fixture (CLAUDE.md Second Priority, ADR-014):"; \
		echo "$$violations"; \
		echo "  Fix: import open_client_session from tools.testing.virtmcu_test_suite.conftest_core"; \
		echo "       and call open_client_session(connect=<endpoint>) instead — or take 'zenoh_session' as a fixture."; \
		echo "       Or, if a non-client-mode session is genuinely required, add inline # ZENOH_OPEN_EXCEPTION: <reason>."; \
		exit 1; \
	fi
	@echo "✓ No raw zenoh.open() in pytest scope."
	@echo "==> Check for hardcoded FDT QOM paths..."
	@if grep -rnE '["'\'']/(flexray|spi[0-9]|wifi[0-9]|uart[0-9]|memory)["'\'']' tests/ ; then \
		echo "❌ ERROR: Hardcoded QOM path without unit address detected. Root FDT devices must use '/device@address' format."; exit 1; \
	fi
	@echo "==> Check for non-deterministic uuid.uuid4() in tests..."
	@# uuid.uuid4() is banned in tests: parallel workers get different IDs each run,
	@# causing Zenoh key collisions and topology mismatches. Use os.getpid(),
	@# the pytest worker_id fixture, or tmp_path-derived values for unique IDs.
	@# Approved exceptions: add # UUID_EXCEPTION: <reason> on the same line.
	@violations=$$(grep -rnE "uuid\.uuid4\(\)" tests/ --include="*.py" \
		| grep -v "fixtures/guest_apps" \
		| grep -v "# UUID_EXCEPTION:" || true); \
	if [ -n "$$violations" ]; then \
		echo "❌ ERROR: Non-deterministic uuid.uuid4() found in tests (use os.getpid()/worker_id instead):"; \
		echo "$$violations"; \
		exit 1; \
	fi
	@echo "✓ No non-deterministic uuid.uuid4() in tests."
	@echo "==> Check for oversized hardcoded timeouts in tests..."
	@# Timeouts >= 200 s in test code bypass the CI time-multiplier and mask deadlocks
	@# on slow/ASan runners. Use vta.step(timeout=T) or bridge.wait_for_line(timeout=T)
	@# with logical T — the framework scales it via get_time_multiplier() automatically.
	@# Approved exceptions: add # TIMEOUT_EXCEPTION: <reason> on the same line.
	@violations=$$(grep -rnE "\btimeout=[2-9][0-9]{2,}|\btimeout=[0-9]{4,}" tests/ --include="*.py" \
		| grep -v "fixtures/guest_apps" \
		| grep -v "# TIMEOUT_EXCEPTION:" || true); \
	if [ -n "$$violations" ]; then \
		echo "❌ ERROR: Oversized hardcoded timeout (>= 200 s) in tests — use get_time_multiplier() scaling:"; \
		echo "$$violations"; \
		exit 1; \
	fi
	@echo "✓ No oversized hardcoded timeouts in tests."
	@$(MAKE) lint-simulation-usage
	@echo "==> ruff check..."
	@uv run --active ruff check .
	@echo "✓ ruff passed."

# Run Simulation Usage Lint
lint-simulation-usage:
	@echo "==> Simulation usage lint..."
	@uv run --active python3 scripts/lint_simulation_usage.py
# Run codespell to catch typos
lint-spelling:
	@echo "==> codespell..."
	@uvx codespell --skip="./third_party/*,./.venv/*,**/build/*,**/target/*,./.git/*,./.claude/*,Cargo.lock,uv.lock,./patches/*,./coverage_report/*,./test-results/*,./.cargo-cache/*,./temp/*" \
		--ignore-words-list="virtmcu,zenoh,qemu,qmp,riscv,TE" .
	@echo "✓ codespell passed."


# Run shellcheck on all bash scripts
lint-shell:
	@echo "==> shellcheck..."
	@shellcheck --version >/dev/null 2>&1 || { echo "❌ Error: shellcheck is not installed. Install with: sudo apt-get install shellcheck"; exit 1; }
	@find . -type f -name "*.sh" -not -path "*/third_party/*" -not -path "*/.venv*" -not -path "*/build/*" -not -path "*/.cargo-cache/*" -print0 | xargs -0 shellcheck --severity=warning
	@echo "==> Checking bash safety flags (set -euo pipefail)..."
	@MISSING=$$(find . -type f -name "*.sh" -not -path "*/third_party/*" -not -path "*/.venv*" -not -path "*/build/*" -not -path "*/.cargo-cache/*" -print0 | xargs -0 grep -rL "set -euo pipefail" 2>/dev/null || true); \
	if [ -n "$$MISSING" ]; then \
		echo "❌ Error: Missing 'set -euo pipefail' in:"; \
		echo "$$MISSING"; \
		exit 1; \
	fi
	@echo "✓ shellcheck passed."


# Run hadolint on Dockerfiles
lint-docker:
	@echo "==> hadolint..."
	@hadolint --version >/dev/null 2>&1 || { echo "❌ Error: hadolint is not installed. Install from: https://github.com/hadolint/hadolint"; exit 1; }
	@hadolint docker/Dockerfile
	@echo "✓ hadolint passed."

# Run actionlint on GitHub Actions workflows
lint-actions:
	@echo "==> actionlint..."
	@actionlint -version >/dev/null 2>&1 || { echo "❌ Error: actionlint is not installed. Install with: (curl -s https://raw.githubusercontent.com/rhysd/actionlint/main/scripts/download-actionlint.bash | bash) && sudo mv actionlint /usr/local/bin/"; exit 1; }
	@actionlint
	@echo "✓ actionlint passed."


# Run yamllint on YAML configuration files
lint-yaml:
	@echo "==> yamllint..."
	@uvx yamllint --strict -d "{extends: relaxed, rules: {line-length: disable}}" $$(find . -type f \( -name "*.yml" -o -name "*.yaml" \) -not -path "*/third_party/*" -not -path "*/.venv*" -not -path "*/build/*" -not -path "*/target/*" -not -path "*/.claude/*" -not -path "*/.cargo-cache/*")
	@echo "✓ yamllint passed."

# Run mypy static type checking
lint-python-types:
	@echo "==> mypy..."
	@uv run --active mypy tools/ tests/ patches/
	@echo "✓ mypy passed."
# Run clang-format and cppcheck on C/C++ files
lint-c:
	@echo "==> clang-format (dry-run)..."
	@clang-format --version >/dev/null 2>&1 || { echo "❌ Error: clang-format is not installed. Install with: sudo apt-get install clang-format"; exit 1; }
	@find hw tools tests -type f \( -name "*.c" -o -name "*.h" -o -name "*.cpp" -o -name "*.cc" -o -name "*.hpp" \) \
		-not -path "*/rust/*" \
		-not -path "*/remote-port/*" \
		-not -path "*/third_party/*" \
		-not -path "*/build/*" \
		-not -path "*/.venv*" \
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


# Run cargo fmt, clippy, and structural checks on Rust files
lint-rust:
	@echo "==> Checking Cargo workspace version synchronization..."
	@cargo metadata --no-deps --format-version 1 | \
		python3 -c "import sys,json; m=json.load(sys.stdin); vs=set(p['version'] for p in m['packages']); assert len(vs)==1, f'version drift: {vs}'"
	@echo "✓ Cargo workspace versions aligned."
	@echo "==> Running cargo fmt --check..."
	@cargo fmt --all --check
	@echo "==> Running cargo machete..."
	@cargo machete
	@echo "==> Checking for banned thread::sleep in hw/rust/..."
	@# thread::sleep is banned in the simulation hot path (MMIO, clock, network callbacks)
	@# because it introduces non-determinism and can starve QEMU of the BQL.
	@# Approved exceptions must carry an inline // SLEEP_EXCEPTION: <reason> comment.
	@# To add an exception: append the comment on the same line as the sleep call.
	@violations=$$(grep -rn "thread::sleep" hw/rust/ --include="*.rs" | grep -v "SLEEP_EXCEPTION:" || true); \
	if [ -n "$$violations" ]; then \
	        echo "ERROR: Banned thread::sleep found in hw/rust/:"; \
	        echo "$$violations"; \
	        echo "  Fix: replace with condvar/channel, or add // SLEEP_EXCEPTION: <reason> inline."; \
	        exit 1; \
	fi
	@echo "==> Checking for stale QEMU plugins..."
	@uv run --active python3 scripts/check-stale-so.py
	@echo "✓ No banned thread::sleep found."
	@echo "==> Checking for banned Mutex<T> in peripheral state structs..."
	@# std::sync::Mutex<T> is banned in zenoh-* peripheral state structs because every
	@# caller already holds the BQL, making the Mutex permanently uncontended and its
	@# presence actively misleading. Use BqlGuarded<T> from virtmcu-qom::sync instead.
	@# Approved exceptions must carry an inline // MUTEX_EXCEPTION: <reason> comment.
	@violations=$$(grep -rn "Mutex<" hw/rust/comms/*/src/lib.rs hw/rust/mcu/*/src/lib.rs hw/rust/observability/*/src/lib.rs | \
		grep -v "Arc<Mutex\|// MUTEX_EXCEPTION:" || true); \
	if [ -n "$$violations" ]; then \
		echo "ERROR: Banned Mutex<T> in peripheral state (use BqlGuarded<T> instead):"; \
		echo "$$violations"; \
		echo "  Fix: replace Mutex<T> with BqlGuarded<T> from virtmcu_qom::sync."; \
		exit 1; \
	fi
	@echo "✓ No banned peripheral Mutex<T> found."
	@echo "==> Checking for banned Bql::lock() and SafeSubscription in hw/rust/comms/..."
	@# Peripherals in hw/rust/comms/ must be independent of the Big QEMU Lock (BQL)
	@# because they interact with asynchronous Zenoh callbacks. Using Bql::lock()
	@# or SafeSubscription (which locks BQL) inside these crates is an anti-pattern
	@# that leads to deadlocks and non-determinism.
	@# Approved exceptions must carry an inline // BQL_EXCEPTION: <reason> comment.
	@violations=$$(grep -rn "Bql::lock()\|SafeSubscription" hw/rust/comms/ | grep -v "BQL_EXCEPTION:" || true); \
	if [ -n "$$violations" ]; then \
		echo "ERROR: Banned BQL usage found in hw/rust/comms/:"; \
		echo "$$violations"; \
		echo "  Fix: remove Bql::lock()/SafeSubscription, or use lock-free channels to communicate with QEMU threads."; \
		exit 1; \
	fi
	@echo "✓ No banned BQL usage in comms found."
	@echo "==> Checking for misleading #![no_std] in hw/rust/..."
	@# #![no_std] is banned in peripheral crates because they implicitly link against std
	@# via virtmcu-qom or zenoh. It provides a false sense of environment safety.
	@# Approved exceptions must carry an inline // NO_STD_EXCEPTION: <reason> comment.
	@violations=$$(grep -rn "#!\[no_std\]" hw/rust/ --include="*.rs" | grep -v "NO_STD_EXCEPTION:" || true); \
	if [ -n "$$violations" ]; then \
		echo "ERROR: Misleading #![no_std] found in hw/rust/:"; \
		echo "$$violations"; \
		echo "  Fix: remove #![no_std] or add // NO_STD_EXCEPTION: <reason> inline."; \
		exit 1; \
	fi
	@echo "✓ No misleading #![no_std] found."
	@echo "==> Checking for banned to_ne_bytes/from_ne_bytes in hw/rust/..."
	@# to_ne_bytes/from_ne_bytes are banned for any value that crosses a process or machine
	@# boundary (socket, Zenoh, shared memory) because they silently corrupt data on
	@# big-endian hosts. Use to_le_bytes/to_be_bytes with a comment stating wire byte order.
	@# to_ne_bytes is permitted only for intra-process data that never leaves the process.
	@# Approved exceptions must carry an inline // NE_BYTES_EXCEPTION: <reason> comment.
	@violations=$$(grep -rn "to_ne_bytes\|from_ne_bytes" hw/rust/ --include="*.rs" | grep -v "NE_BYTES_EXCEPTION:" || true); \
	if [ -n "$$violations" ]; then \
		echo "ERROR: Banned to_ne_bytes/from_ne_bytes found in hw/rust/:"; \
		echo "$$violations"; \
		echo "  Fix: use to_le_bytes()/from_le_bytes() with a wire-order comment, or add // NE_BYTES_EXCEPTION: <reason>."; \
		exit 1; \
	fi
	@echo "✓ No banned to_ne_bytes/from_ne_bytes found."
	@echo "==> Checking for banned rand::thread_rng in hw/rust/..."
	@# rand::thread_rng() is banned in simulation code because it seeds from wall-clock
	@# entropy, breaking determinism. Use seed_for_quantum(global_seed, node_id, quantum)
	@# from transport-zenoh for all stochastic simulation behaviour.
	@# Approved exceptions must carry an inline // RNG_EXCEPTION: <reason> comment.
	@violations=$$(grep -rn "rand::thread_rng\b" hw/rust/ --include="*.rs" | grep -v "RNG_EXCEPTION:" || true); \
	if [ -n "$$violations" ]; then \
		echo "ERROR: Banned rand::thread_rng found in hw/rust/:"; \
		echo "$$violations"; \
		echo "  Fix: use seed_for_quantum() from transport-zenoh, or add // RNG_EXCEPTION: <reason>."; \
		exit 1; \
	fi
	@echo "✓ No banned rand::thread_rng found."
	@echo "==> Checking for banned #[allow(] in hw/rust/ production code..."
	@# #[allow(...)] suppresses clippy/rustc diagnostics and is banned in production code.
	@# All = "deny" in [workspace.lints.clippy] means every warning is already an error;
	@# an allow annotation silently creates a hole in that guarantee.
	@# Fix the underlying issue instead. Test-only exceptions (#[cfg(test)] scope) are
	@# permitted; document the reason with // ALLOW_EXCEPTION: <reason> on the same line.
	@violations=$$(grep -rn "#\[allow(" hw/rust/ --include="*.rs" \
		--exclude-dir=target \
		--exclude-dir=tests \
		--exclude="*_generated.rs" \
		--exclude="build.rs" \
		| grep -v "// ALLOW_EXCEPTION:" || true); \
	if [ -n "$$violations" ]; then \
		echo "ERROR: Banned #[allow(] found in hw/rust/ — fix the underlying lint instead:"; \
		echo "$$violations"; \
		exit 1; \
	fi
	@echo "✓ No banned #[allow(] in hw/rust/."
	@echo "==> Checking QOM TypeInfo / DTS / Meson name alignment..."
	@# A QOM type name in Rust TypeInfo MUST match (a) the meson.build 'obj' field
	@# for that crate, (b) the DTS 'compatible' string used by integration tests,
	@# and (c) the -global prefix used in test extra_args. Gemini's flexray crash
	@# (rc=-11) was a NULL-deref inside QEMU's error_prepend triggered by a
	@# 4-way mismatch (lib.rs c"virtmcu,flexray", DTS "flexray",
	@# meson 'obj': 'flexray', test -global flexray.*).
	@uv run --active python3 scripts/check-qom-alignment.py || exit 1
	@echo "✓ QOM TypeInfo / DTS / Meson alignment OK."
	@echo "==> Checking Cargo lib name vs Meson 'lib' field..."
	@# Each Rust peripheral package's static lib output must match the 'lib' field
	@# in third_party/qemu/hw/virtmcu/meson.build. A mismatch causes Meson to link
	@# a stale .a file, producing a working-looking .so that contains old code.
	@# Detection rule: for each crate under hw/rust/{comms,mcu,observability,backbone},
	@# the package name's underscore form must equal the meson 'lib' field stripped
	@# of 'lib' prefix and '.a' suffix.
	@uv run --active python3 scripts/check-cargo-meson-lib-alignment.py || exit 1
	@echo "✓ Cargo / Meson library names aligned."
	@echo "==> Running cargo clippy..."
	@cargo clippy --workspace -- -D warnings

# Run meson format check
lint-meson:
	@echo "==> Running meson format..."
	@uvx meson format -q hw/meson.build
	@echo "✓ meson format passed."
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

# Run all formatters
fmt: fmt-python fmt-rust fmt-meson fmt-c fmt-yaml

# Format Python files using ruff
fmt-python: venv
	@echo "==> ruff format + fix..."
	uv run --active ruff format .
	uv run --active ruff check . --fix

# Format Rust files using cargo fmt
fmt-rust:
	@echo "==> cargo fmt..."
	@cargo fmt --all

# Format Meson build files
fmt-meson:
	@echo "==> meson format..."
	@meson fmt -i hw/meson.build && echo "✓ meson format passed." || { echo "❌ meson format failed"; exit 1; }

# Format C/C++ files using clang-format
fmt-c:
	@echo "==> clang-format..."
	@find hw -type f \( -name "*.c" -o -name "*.h" \) -not -path "*/rust/*" -not -path "*/third_party/*" -print0 | xargs -0 clang-format -i && echo "✓ clang-format passed." || { echo "❌ clang-format failed"; exit 1; }

# Format YAML files (strip trailing whitespace)
fmt-yaml:
	@echo "==> stripping trailing whitespace from YAMLs..."
	@find . -type f \( -name "*.yml" -o -name "*.yaml" \) -not -path "*/third_party/*" -not -path "*/.venv*" -print0 | xargs -0 sed -i 's/[[:space:]]*$$//'

# Install pre-commit and pre-push git hooks
install-hooks:
	@echo "==> Installing Git hooks..."
	@mkdir -p .git/hooks
	# Hooks run directly in the current environment (devcontainer or native).
	# We set PYTEST_WORKERS=4 to provide parallelism while remaining pipe-safe.
	# Standard stdin is redirected from /dev/null to prevent interactive hangs.
	@printf '#!/bin/sh\nset -e\nPYTEST_WORKERS=4 make lint < /dev/null && PYTEST_WORKERS=4 make test-unit < /dev/null\n' > .git/hooks/pre-commit
	@printf '#!/bin/sh\nset -e\nPYTEST_WORKERS=4 make lint < /dev/null && PYTEST_WORKERS=4 make test-unit < /dev/null\n' > .git/hooks/pre-push
	@chmod +x .git/hooks/pre-push .git/hooks/pre-commit
	@echo "✓ hooks installed: pre-commit and pre-push run 'make lint && make test-unit' directly."

# ------------------------------------------------------------------------------
# Performance Benchmarking & Trend Tracking (boot_arm6)
# ------------------------------------------------------------------------------

# Run the full performance benchmark and save results to tests/fixtures/guest_apps/perf_bench/last_results.json.
perf-bench: venv
	@$(MAKE) -C tests/fixtures/guest_apps/perf_bench bench.elf
	PYTHONPATH=$(CURDIR) uv run --active python3 tests/fixtures/guest_apps/perf_bench/bench.py

# Save the current benchmark results as the performance baseline.
perf-baseline: perf-bench
	uv run --active python3 scripts/perf_trend.py --save-baseline
	@echo "✓ Performance baseline updated."

# Check current benchmark results against the saved baseline; exit 1 on regression.
perf-check: venv
	@if [ ! -f tests/fixtures/guest_apps/perf_bench/last_results.json ]; then \
		$(MAKE) -C tests/fixtures/guest_apps/perf_bench bench.elf && PYTHONPATH=$(CURDIR) uv run --active python3 tests/fixtures/guest_apps/perf_bench/bench.py; \
	fi
	uv run --active python3 scripts/perf_trend.py --check

# ------------------------------------------------------------------------------
# Local CI Simulation
# Mirrors the GitHub CI tier structure so you can validate locally before push.
#
# Three escalating gates — each one is a strict superset of the previous:
#
#   Hooks (pre-commit / pre-push)
#     make lint && make test-unit   — run directly in the devcontainer.
#     Fast (~3-5 min). No Docker spawn. Use --no-verify to skip in an emergency.
#
#   make ci-local   — GitHub Tier 1 simulation in a fresh devenv-base container.
#     Runs lint → build-tools → test-unit in the SAME image and with the SAME
#     flags that .github/workflows/ci-main.yml uses.  No CARGO_HOME override.
#     Run this before opening a pull request.
#
#   make ci-smoke DOMAIN=N — Run a single CI integration domain inside the builder Docker image.
#     Perfect for testing exactly what GitHub will run for a specific integration test.
#
#   make ci-full    — Full pipeline: ci-local + ci-asan + ci-miri + builder image
#     (full QEMU compile) + all integration domains run sequentially inside the builder.
#     This is the authoritative "will GitHub be green?" answer.
#     Run this before merging to main.
#
# Mount strategy for all 'docker run devenv-base' invocations:
#
#   /workspace (bind-mount $(CURDIR))
#     Source code + generated Python .venv-docker + test-results.
#     The host's own 'target/' directory is NOT used by the container.
#
#   CARGO_TARGET_DIR=/tmp/ci-target  (ephemeral inside the container)
#     Compiled Rust artifacts stay inside the container and disappear when it
#     exits.  This is the critical fix: sharing target/ between environments
#     with different CARGO_HOME values corrupts Cargo's fingerprint cache and
#     causes "can't find crate" errors on proc-macro crates.
#
#   ci-cargo-registry (named Docker volume → /usr/local/cargo/registry)
#     Crate source tarballs are cached in a named volume that persists across
#     'docker run' calls without ever touching the host filesystem.  The
#     container's own /usr/local/cargo is used (matching GitHub CI exactly —
#     GitHub's docker run also carries no CARGO_HOME override).
#
#   UV_PROJECT_ENVIRONMENT=/workspace/.venv-docker
#     Python venv isolated from the host's .venv so tool versions don't bleed.
# ------------------------------------------------------------------------------

# Run the local equivalent of CI Tier 1 (Linting, build, unit tests)
ci-local:
	@echo "════════════════════════════════════════════════════"
	@echo "  CI Tier 1 — GitHub-identical: lint → build-tools → test-unit"
	@echo "════════════════════════════════════════════════════"
	@bash scripts/docker-build.sh devenv-base
	@echo ""
	# Ensure the cargo registry named volume is owned by the current user.
	# Docker initialises new named volumes from the image layer (root-owned),
	# so --user runs would fail on first crate download without this step.
	docker run --rm -v ci-cargo-registry:/vol $(DEVENV_BASE_IMG) \
		sh -c "chown -R $$(id -u):$$(id -g) /vol"
	# Mirrors the three 'docker run devenv-base' steps from .github/workflows/ci-main.yml.
	# CARGO_HOME is NOT overridden — container uses its baked-in /usr/local/cargo,
	# exactly as GitHub CI does.  See mount strategy comment above for full details.
	docker run --rm \
		--user $$(id -u):$$(id -g) \
		-e HOME=/tmp \
		-e USER=vscode \
		-e CI=true \
		-e CARGO_TARGET_DIR=/tmp/ci-target \
		-e VIRTMCU_SKIP_QEMU_HEADERS_WARNING=1 \
		-e UV_PROJECT_ENVIRONMENT=/workspace/.venv-docker \
		-v "$(CURDIR):/workspace" \
		-v ci-cargo-registry:/usr/local/cargo/registry \
		-w /workspace \
		$(DEVENV_BASE_IMG) bash -c "make lint && make build-tools && make test-unit"
	@echo ""
	@echo "✓ ci-local passed (1:1 with GitHub CI Tier 1)."
	@echo "  To run the full pipeline (builder ~40 min + all integration domains): make ci-full"

# Run host-side C coverage for peripheral plugins (inside builder)
test-coverage-peripheral:
	@echo "==> Running peripheral C coverage (gcovr)..."
	@bash scripts/docker-build.sh builder
	@mkdir -p test-results
	@docker run --rm \
		-v "$(CURDIR):/workspace" -w /workspace \
		-e CI=true \
		$(BUILDER_IMG) \
		bash -c "gcovr -r /build/qemu/hw/virtmcu \
			--gcov-executable gcov \
			--object-directory /build/qemu/build-virtmcu \
			--object-directory /workspace/coverage-data \
			--xml /workspace/test-results/peripheral-coverage.xml \
			--html-details /workspace/test-results/peripheral-coverage.html \
			--print-summary"

# Run the full pipeline: ci-local + ci-asan + ci-miri + builder image tests
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
		-e CI=true \
		-e VIRTMCU_STALL_TIMEOUT_MS=120000 \
		-e GCOV_PREFIX=/workspace/coverage-data \
		-e GCOV_PREFIX_STRIP=3 \
		-e VIRTMCU_SKIP_BUILD_DIR=1 \
		$(BUILDER_IMG) \
		bash scripts/ci-integration-tier.sh all
	@echo ""
	@echo "════════════════════════════════════════════════════"
	@echo "  CI Full — Coverage Checks"
	@echo "════════════════════════════════════════════════════"
	$(MAKE) test-coverage-guest
	$(MAKE) test-coverage-peripheral
	@echo ""
	@echo "✓ ci-full passed."

# Run a single CI integration domain in Docker (e.g., make ci-smoke DOMAIN=2)
ci-smoke:
	@if [ -z "$(DOMAIN)" ]; then \
		echo "Usage: make ci-smoke DOMAIN=<domain_name>"; \
		echo "Example: make ci-smoke DOMAIN=2"; \
		exit 1; \
	fi
	@echo "════════════════════════════════════════════════════"
	@echo "  CI Integration Domain $(DOMAIN) — Docker: builder"
	@echo "════════════════════════════════════════════════════"
	@bash scripts/docker-build.sh builder
	@mkdir -p coverage-data
	docker run --rm \
		-v "$(CURDIR):/workspace" -w /workspace \
		-e PYTHONPATH=/workspace \
		-e CI=true \
		-e VIRTMCU_STALL_TIMEOUT_MS=120000 \
		-e GCOV_PREFIX=/workspace/coverage-data \
		-e GCOV_PREFIX_STRIP=3 \
		-e VIRTMCU_SKIP_BUILD_DIR=1 \
		$(BUILDER_IMG) \
		bash scripts/ci-integration-tier.sh $(DOMAIN)

# Run integration tests compiled with ASan/UBSan inside devenv-base
ci-asan:
	@echo "════════════════════════════════════════════════════"
	@echo "  CI ASan — Docker: devenv-base"
	@echo "════════════════════════════════════════════════════"
	@bash scripts/docker-build.sh devenv-base
	@echo ""
	@echo "════════════════════════════════════════════════════"
	@echo "  CI ASan — Building and testing under ASan"
	@echo "════════════════════════════════════════════════════"
	docker run --rm -v ci-cargo-registry:/vol $(DEVENV_BASE_IMG) \
		sh -c "chown -R $$(id -u):$$(id -g) /vol"
	docker run --rm \
		--user $$(id -u):$$(id -g) \
		-e HOME=/tmp \
		-e USER=vscode \
		-e CI=true \
		-e CARGO_TARGET_DIR=/tmp/ci-target \
		-e VIRTMCU_SKIP_QEMU_HEADERS_WARNING=1 \
		-e UV_PROJECT_ENVIRONMENT=/workspace/.venv-docker \
		-e VIRTMCU_STALL_TIMEOUT_MS=300000 \
		-v "$(CURDIR):/workspace" \
		-v ci-cargo-registry:/usr/local/cargo/registry \
		-w /workspace \
		$(DEVENV_BASE_IMG) make test-asan
	@echo ""
	@echo "✓ ci-asan passed."

# Run Miri tests to detect Undefined Behavior inside devenv-base
ci-miri:
	@echo "════════════════════════════════════════════════════"
	@echo "  CI Miri — Docker: devenv-base"
	@echo "════════════════════════════════════════════════════"
	@bash scripts/docker-build.sh devenv-base
	@echo ""
	@echo "════════════════════════════════════════════════════"
	@echo "  CI Miri — Running Miri tests"
	@echo "════════════════════════════════════════════════════"
	docker run --rm -v ci-cargo-registry:/vol $(DEVENV_BASE_IMG) \
		sh -c "chown -R $$(id -u):$$(id -g) /vol"
	docker run --rm \
		--user $$(id -u):$$(id -g) \
		-e HOME=/tmp \
		-e USER=vscode \
		-e CI=true \
		-e CARGO_TARGET_DIR=/tmp/ci-target \
		-v "$(CURDIR):/workspace" \
		-v ci-cargo-registry:/usr/local/cargo/registry \
		-w /workspace \
		$(DEVENV_BASE_IMG) make test-miri
	@echo ""
	@echo "✓ ci-miri passed."

# ------------------------------------------------------------------------------
# Docker Image Targets
# ------------------------------------------------------------------------------
# All versions are read from the BUILD_DEPS file by scripts/docker-build.sh.
# Pass IMAGE_TAG=<tag> to override the local tag (default: dev).
#
#   make docker-dev    — base → toolchain → devenv with smoke tests (fast path)
#   make docker-all    — full pipeline including builder (~40 min) and runtime
#   make docker-base   — build a single stage (no smoke test, for debugging)

# Build docker base -> toolchain -> devenv with smoke tests
docker-dev:
	@bash scripts/docker-build.sh dev

# Build all docker stages including builder and runtime
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

# Build only the docker builder stage
docker-builder:
	@bash scripts/docker-build.sh builder

# Build only the docker runtime stage
docker-runtime:
	@bash scripts/docker-build.sh runtime

# ------------------------------------------------------------------------------
# Release
# ------------------------------------------------------------------------------
# Create an annotated git tag, record the version in VERSION, and push both
# the commit and the tag.  GitHub CI then publishes versioned container images
# (devenv:vMAJOR.MINOR.PATCH, runtime:vMAJOR.MINOR.PATCH, per-arch variants)
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
	rm -rf .pytest_cache .ruff_cache
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
	@echo "✓ Clean complete (QEMU sources and .venv remain)."

# Deep clean: completely remove downloaded sources, virtual environments, and all artifacts.
# You will need to run 'make setup-initial' again after this.
distclean: clean
	rm -rf .venv
	rm -rf third_party
	rm -rf test-results
	@echo "✓ Deep clean complete. Run 'make setup-initial' to rebuild the environment."
# ------------------------------------------------------------------------------
# Documentation
# ------------------------------------------------------------------------------

# Build the mdBook documentation
book:
	@echo "==> Building mdBook..."
	@if command -v mdbook >/dev/null 2>&1; then \
		mdbook build; \
	else \
		echo "❌ mdbook not installed. Please restart devcontainer or run: cargo install mdbook"; exit 1; \
	fi
	@echo "✓ mdBook built in target/book."

# Serve the mdBook documentation locally (uses Python to avoid WebSocket/DevContainer port forwarding issues)
book-serve: book
	@echo "==> Serving mdBook..."
	@echo "    Click this link to open: http://localhost:8080"
	@python3 -m http.server -d target/book 8080

