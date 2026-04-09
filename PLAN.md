# qenode Implementation Plan

**Goal**: Make QEMU behave like Renode — dynamic device loading, FDT-based ARM machine
instantiation, .repl parsing, and Robot Framework test parity.

**Base**: QEMU 11.0.0-rc2 + 33-patch arm-generic-fdt series (patchew 20260402215629)
**Target arch**: ARM (Cortex-A / Cortex-M) first; RISC-V deferred to Phase 2+
**Dev platform**: Linux required (Docker/WSL2 on macOS/Windows)

---

## Phase 0 — Repository Setup ✅

**Status**: Done

### Tasks
- [x] Directory scaffold: `hw/`, `tools/repl2qemu/`, `tools/testing/`, `scripts/`, `docs/`
- [x] `CLAUDE.md` — AI agent context file (architecture decisions, constraints, local paths)
- [x] `PLAN.md` — this file
- [x] `README.md` — human-readable overview
- [x] `docs/ARCHITECTURE.md` — consolidated QEMU vs Renode analysis (replaces the two
      duplicate .md files; Antigravity IDE artifacts removed)
- [x] `.gitignore` updated for `modules/`, `build/`, `*.so`, `*.dtb`, `.venv/`

---

## Phase 1 — QEMU Build with arm-generic-fdt ⬜

**Goal**: A working QEMU binary on Linux with `--enable-modules` and the arm-generic-fdt
machine type. Validates that the patch series applies cleanly and FDT-based boot works.

**Acceptance criteria**:
- `qemu-system-arm -M arm-generic-fdt -hw-dtb minimal.dtb -nographic` starts and
  reaches the kernel entry point (verified via `-d exec,cpu_reset`).
- `qemu-system-arm -device help` lists `arm-generic-fdt` as a valid machine.

### Tasks
- [ ] **1.1** Write `scripts/setup-qemu.sh`:
  - Confirm QEMU is at `~/src/qemu` and at v10.2.92 / 11.0.0-rc2
  - Apply the 33-patch arm-generic-fdt series from local mailbox `patches/arm-generic-fdt-v3.mbx` via `git am --3way`
  - Apply the libqemu external time master patch via `python3 patches/apply_libqemu.py`
  - Configure: `../configure --enable-modules --enable-fdt --enable-plugins --enable-debug
      --target-list=arm-softmmu,arm-linux-user --prefix=$(pwd)/install`
  - Build: `make -j$(nproc)`

- [ ] **1.2** Write a minimal `test/phase1/minimal.dts` for the arm-generic-fdt machine:
  - Single Cortex-A15 CPU, 128 MB RAM, PL011 UART at 0x09000000
  - Compile: `dtc -I dts -O dtb -o minimal.dtb minimal.dts`

- [ ] **1.3** Write `scripts/run.sh` skeleton:
  - Accepts `--dtb`, `--kernel`, `--machine` args
  - Sets `QEMU_MODULE_DIR` to the library output directory
  - Execs `qemu-system-arm` with those environment variables

- [ ] **1.4** Smoke-test: boot the minimal DTB, verify UART output reaches host terminal.

---

## Phase 2 — Dynamic QOM Plugin Infrastructure ⬜

**Goal**: Compile a minimal out-of-tree QOM peripheral as a `.so`, load it into QEMU
via native module discovery + `scripts/run.sh`, and confirm the type appears in QOM.

**Acceptance criteria**:
- `./scripts/run.sh --dtb test/phase1/minimal.dtb -device dummy-device` starts QEMU
  without "unknown device" error.
- `info qom-tree` in QEMU monitor shows `dummy-device` attached.

### Tasks
- [ ] **2.1** Write `hw/dummy/dummy.c` — minimal correct QOM SysBusDevice:
  - Include `qemu/osdep.h` first (always), then `hw/sysbus.h`
  - Use `OBJECT_DECLARE_SIMPLE_TYPE(DummyDevice, DUMMY_DEVICE)`
  - Use `DEFINE_TYPES(dummy_types)` (QEMU 7+ pattern, not `type_register_static`)
  - Implement MMIO read/write stubs (return 0, log access via `qemu_log_mask`)
  - No `#define BUILD_DSO` — this is not a QEMU macro

- [ ] **2.2** Update QEMU module build configuration:
  - Add symlink to link `hw/` into QEMU's source tree
  - Add `hw/meson.build` to define `hw_qenode_modules`
  - Output: `hw-qenode-dummy.so` within QEMU's installed `lib/qemu/`

- [ ] **2.3** Verify the native module loading:
  - `./scripts/run.sh -machine none -device dummy-device`
  - Should auto-load `dummy-device` and print type registration trace, not "unknown device"

- [ ] **2.4** Add a Rust template (optional, lower priority):
  - Crate in `hw/rust-dummy/` using `qemu-plugin` crate or raw FFI
  - Demonstrates the C/Rust peripheral interop story

**Known issue**: QEMU headers require GLib. On some distros you need `libglib2.0-dev`.
The build script should check for this and provide a clear error message.

---

## Phase 3 — repl2qemu Parser ⬜

**Goal**: Parse a real Renode `.repl` file (STM32F4 Discovery or similar) and produce
a valid `.dtb` file that arm-generic-fdt can boot with.

**Acceptance criteria**:
- `python -m tools.repl2qemu stm32f4_discovery.repl` produces `out.dtb` and prints
  the equivalent QEMU CLI command.
- `qemu-system-arm -M arm-generic-fdt -hw-dtb out.dtb` successfully reaches the reset
  handler for a simple Zephyr blinky firmware.

### Tasks
- [ ] **3.1** Obtain reference `.repl` files from Renode's public repo:
  - `~/src/renode/platforms/cpus/stm32f4.repl` (Cortex-M4, STM32)
  - A Zynq or Cortex-A based board for arm-generic-fdt validation
  - Check: `ls ~/src/renode/platforms/`

- [ ] **3.2** Write `tools/repl2qemu/parser.py`:
  - Grammar (Lark EBNF) covering:
    - Indent-mode device blocks: `name: ClassName @ sysbus <address>`
    - Properties: `key: value` / `key: "string"` / `key: <ref>`
    - Interrupts: `-> target@line`
    - `using` includes
  - AST node types: `Platform`, `Device`, `Property`, `Interrupt`, `Include`

- [ ] **3.3** Write `tools/repl2qemu/fdt_emitter.py`:
  - Walk AST → emit DTS text
  - Map Renode types to DTS `compatible` strings:
    - `UART.PL011` → `"arm,pl011"`
    - `Memory.MappedMemory` → DTS `memory@<addr>` node
    - `Timers.ARM_GenericTimer` → `"arm,armv8-timer"`
    - Interrupts: map `-> gic@0` to `interrupts = <GIC_SPI N IRQ_TYPE_LEVEL_HIGH>`
  - Invoke `dtc` via subprocess to compile DTS → DTB

- [ ] **3.4** Write `tools/repl2qemu/cli_generator.py`:
  - Walk AST → build QEMU CLI arg list
  - Map `.resc` commands:
    - `sysbus LoadELF $bin` → `-kernel $bin`
    - `machine StartGdbServer 3333` → `-gdb tcp::3333 -S`
    - `machine EnableProfiler` → `-d exec`

- [ ] **3.5** Write `tools/repl2qemu/__main__.py` (CLI entry point):
  - `python -m tools.repl2qemu input.repl [--out-dtb out.dtb] [--print-cmd]`

- [ ] **3.6** Unit tests in `tests/test_parser.py`:
  - Test tokenizer on known .repl snippets
  - Test DTS output for a 3-device platform

**Needs from Marcin**:
- Confirm whether you have proprietary `.repl` files to test against edge cases.
  If so, share sanitized examples during this phase.

---

## Phase 4 — Robot Framework QMP Library ⬜

**Goal**: A `.robot` resource file that provides Renode-compatible test keywords backed
by QEMU's QMP protocol, enabling existing Robot Framework test suites to run on QEMU
with minimal keyword substitution.

**Acceptance criteria**:
- A Robot Framework test that uses `Start Emulation`, `Wait For Line On UART`,
  `PC Should Be Equal`, and `Reset Emulation` passes against a running QEMU instance.

### Tasks
- [ ] **4.1** Write `tools/testing/qmp_bridge.py`:
  - Async wrapper around `qemu.qmp` library
  - `connect(socket_path)`, `execute(cmd, args)`, `wait_for_event(event_name)`
  - UART monitoring: connect to QEMU chardev socket, non-blocking readline
  - Use `query-cpus-fast` (NOT deprecated `query-cpus`)

- [ ] **4.2** Write `tools/testing/qemu_keywords.robot`:
  - `Start Emulation` → `{"execute": "cont"}`
  - `Reset Emulation` → `{"execute": "system_reset"}`
  - `Pause Emulation` → `{"execute": "stop"}`
  - `PC Should Be Equal  ${addr}` → `query-cpus-fast`, assert `pc` field
  - `Wait For Line On UART  ${pattern}  ${timeout}` → chardev socket regex read
  - `Execute Monitor Command  ${cmd}` →
    `{"execute": "human-monitor-command", "arguments": {"command-line": "${cmd}"}}`
  - `Load ELF  ${path}` → pre-boot only; handled by CLI generator (not QMP)

- [ ] **4.3** Write `tools/testing/conftest.py` (pytest fixtures for QMP tests)

- [ ] **4.4** Integration test `tests/test_qmp_keywords.robot`:
  - Start QEMU with minimal DTB + simple bare-metal ELF (prints "HELLO" to UART)
  - `Wait For Line On UART  HELLO  timeout=10`
  - Assert pass

---

## Phase 5 — Co-Simulation Bridge ⬜ (Deferred)

**Prerequisite**: Phases 1-4 complete and validated.

**Goal**: Enable SystemC peripheral models to connect to QEMU. Three paths are available
(see `docs/ARCHITECTURE.md` §9 for the full decision guide):

- **Path A** (chardev socket, available now): thin C++ adapter translates TLM transactions
  to qenode's Unix socket protocol. Works for individual peripherals at <1 MHz access rate.
- **Path B** (Remote Port, this phase): full TLM-2.0 co-simulation via AMD/Xilinx Remote
  Port. Required for Verilated FPGA fabric / complex SoC subsystems.
- **Path C** (qbox, future): adopt Qualcomm qbox's `libqemu-cxx` for tight TLM embedding.

**Source of Verilated models**: Any Verilated C++ models will come from Renode's
existing co-simulation setup (Renode's `CoSimulationPlugin` / `IntegrationLibrary`).
Migration means replacing those Renode headers with qenode's Remote Port interface.

**EtherBone (FPGA over UDP)**: Nice-to-have for Renode feature parity, not P0.
Implement after Path B is validated.

### Tasks
- [ ] **5.1** Implement Path A: write `tools/systemc_adapter/` — C++ shim translating
      QEMU chardev socket messages to SystemC TLM-2.0 `b_transport` calls. Validate
      with a simple register-file model. *(No Verilated models needed to start.)*
- [ ] **5.2** Implement Path B: strip Renode `IntegrationLibrary` headers from existing
      Verilated models; integrate `libsystemctlm-soc`; write `hw/remote-port/` QOM device;
      validate end-to-end with one Renode-derived Verilated model.
- [ ] **5.3** *(P2)* Write `hw/etherbone/etherbone-bridge.c` — MMIO → UDP for FPGA-over-network.
- [ ] **5.4** Document Path A vs B vs C decision guide (already in `docs/ARCHITECTURE.md` §9).

---

## Phase 7 — FirmwareStudio / MuJoCo External Time Master ⬜ (Future)

**Goal**: qenode becomes the QEMU layer of FirmwareStudio. MuJoCo drives physical
simulation; its `TimeAuthority` class advances QEMU's virtual clock one quantum at a time
over Zenoh, guaranteeing causal consistency between physics and firmware.

**Background**: FirmwareStudio (`~/src/FirmwareStudio`) already has a working prototype:
- `physics/time_authority/` — Python `TimeAuthority` class running in MuJoCo container
- `cyber/patches/0001-add-libqemu-clocksock.patch` — QEMU patch that exposes a Unix socket
- `cyber/src/node_agent.py` — bridges Zenoh ↔ QEMU Unix socket
- `cyber/src/shm_bridge.py` — bridges IVSHMEM MMIO ↔ Zenoh for sensor/actuator I/O

qenode's job in Phase 7: replace the prototype with production-quality implementations.

### Three Clock Modes

| Mode | QEMU flag | Performance | Use when |
|---|---|---|---|
| `standalone` | (none) | 100% | Development, CI without physics |
| `slaved-suspend` | `-clocksock` optional | ~95% | FirmwareStudio default — QMP stop/cont at boundaries |
| `slaved-icount` | `-clocksock -icount shift=0,align=off,sleep=off` | ~15–20% | Sub-quantum timing needed (PWM, hardware timers) |

Inspired by Qualcomm qbox's `libgssync` suspend/resume pattern:
`slaved-suspend` uses QMP `stop`/`cont` at quantum boundaries — no icount penalty,
full TCG speed within each step. The ±1 quantum jitter is irrelevant for control loops.

### Design: External Time Master Protocol

```
MuJoCo (mj_step)
    → TimeAuthority.step(quantum_ns)
        → Zenoh: GET sim/clock/advance/{node_id}  payload=(delta_ns, mujoco_time_ns)
            → NodeAgent (Python, runs beside QEMU)
                → Unix socket: ClockAdvance{delta_ns, mujoco_time_ns}
                    → QEMU (libqemu patch): qemu_icount_bias += delta_ns
                    → QEMU runs exactly delta_ns ns of virtual time, then blocks
                    → QEMU replies: ClockReady{vtime_ns, n_frames} + Ethernet frames
                ← NodeAgent collects reply + frames
            ← Zenoh reply: ClockReady + frames
        ← TimeAuthority routes Ethernet frames between nodes
```

**icount mode** must be configured: `-icount shift=0,align=off,sleep=off`
This ensures QEMU advances exactly the requested number of nanoseconds with no drift.

### Performance of the External Clock Approach

The stepping protocol adds latency per quantum (typically 1–10 ms of sim time per step):
- Zenoh round-trip (same machine, Unix socket backend): ~10–50 µs
- QEMU icount advance (no actual CPU execution if delta is small): ~1 µs
- For a 1 kHz physics loop (1 ms quantums): overhead is < 5% of wall time

The bottleneck is **not** the clock stepping — it is QEMU executing firmware instructions
within each quantum. icount mode disables JIT optimisations like TB chaining, so raw
instruction throughput drops by ~5–10× compared to QEMU's default mode. For typical
bare-metal firmware (Cortex-A15, 100 MHz effective, simple control loops), this is
plenty fast enough for 1 kHz and even 10 kHz physics loops.

### Tasks
- [x] **7.1** Implement `patches/apply_libqemu.py` to systematically inject `-clocksock` support:
  - Replaces fragile .patch files with AST-aware code injection
  - Handled by `scripts/setup-qemu.sh`

- [ ] **7.2** Write `tools/node_agent/node_agent.py`:
  - Production-quality port of FirmwareStudio's `node_agent.py`
  - Configurable Zenoh router address, node ID, socket path
  - Handle frame injection (Ethernet frames from TimeAuthority → QEMU)

- [ ] **7.3** Write `tools/node_agent/qemu_clock.py`:
  - Clean abstraction over the Unix socket protocol (`ClockAdvance`/`ClockReady`)
  - Async (asyncio-compatible)

- [ ] **7.4** Integration test: boot minimal firmware, step 1000 × 1 ms, assert
  firmware timestamps are deterministic across two identical runs.

- [ ] **7.5** Replace FirmwareStudio's `cyber/` with a dependency on qenode:
  - `worlds/*.yml` Docker Compose files reference qenode's patched QEMU image
  - `cyber/src/` → qenode `tools/node_agent/`

---

## Phase 6 — Multi-Node Coordination ⬜ (Future)

**Goal**: Python coordinator script to orchestrate multiple QEMU instances with simulated
wireless medium (packet loss/latency by distance), replacing Renode's `WirelessMedium`.

- Each QEMU instance: `-netdev socket,id=net0,mcast=230.0.0.1:1234`
- Coordinator intercepts multicast packets, applies attenuation model, rebroadcasts
- Determinism achieved by running with `icount` mode and locking virtual time

---

## Risks and Open Questions

| # | Risk | Mitigation |
|---|------|-----------|
| R1 | arm-generic-fdt patchew series may not apply cleanly to v10.2.92 HEAD | Pin to the exact commit the patchew was submitted against; cherry-pick conflicts manually |
| R2 | Native module approach fails on some macOS builds | Omit `--enable-plugins` on Darwin natively to bypass GLib symbol conflict |
| R3 | macOS `.so` loading is broken with `--enable-plugins` | Enforce Linux-only dev environment in CI |
| R4 | vhost-user Python daemons add IPC latency | Reserve for low-speed peripherals (<1 MHz); profile before using in tight interrupt loops |
| R5 | Renode .repl parser has undocumented edge cases | Use Renode source (`~/src/renode`) as ground truth; diff parser output against Renode's own AST |
| R6 | `arm-generic-fdt` v3 patch series may have changed between patchew submission and merger | Track patchew thread; re-fetch if a v4 series is posted |
| R7 | icount mode reduces firmware execution speed ~5–10× | Acceptable for control loops ≤10 kHz; profile with `perf` if needed |
| R8 | FirmwareStudio `libqemu` patch uses placeholder git hashes (aaaa/bbbb) and may not apply | Must be manually rewritten with real context lines against QEMU 11.0.0-rc2 |

---

## Deferred / Won't Do (Phase 1-4 scope)

- Windows support (module loading fundamentally broken on Windows with current QEMU)
- RISC-V until ARM is validated
- RESD (Renode Sensor Data) format injection
- Antigravity IDE / agent_memory.json / mcp_servers.json (not project artifacts)
- `query-cpus` (deprecated — use `query-cpus-fast` only)
