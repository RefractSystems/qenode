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
  - Confirm QEMU is loaded in `third_party/qemu` and at v10.2.92 / 11.0.0-rc2
  - Apply the 33-patch arm-generic-fdt series from local mailbox `patches/arm-generic-fdt-v3.mbx` via `git am --3way`
  - Apply the libqemu external time master patch via `python3 patches/apply_libqemu.py`
  - Apply the TCG quantum hook patch via `python3 patches/apply_zenoh_hook.py` (exposes a function pointer in `cpu-exec.c` since QOM devices cannot hook the TCG loop natively)
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
  - `third_party/renode/platforms/cpus/stm32f4.repl` (Cortex-M4, STM32)
  - A Zynq or Cortex-A based board for arm-generic-fdt validation
  - Check: `ls third_party/renode/platforms/`

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
  - Map `--native-accel` arguments (see ADR-007):
    - If AST indicates Cortex-A and running on ARM host → append `-accel kvm` (Linux) or `-accel hvf` (Mac)
    - If AST indicates Cortex-M → append `-accel tcg`

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

**Goal**: A test automation layer that provides Renode-compatible coverage backed by QEMU's
QMP protocol. Primary implementation is pytest + `qemu.qmp`; a `.robot` resource file is
maintained for compatibility with existing Renode test suites.

**Acceptance criteria**:
- A pytest test using `QmpBridge` that calls `Start Emulation`, `Wait For Line On UART`,
  `PC Should Be Equal`, and `Reset Emulation` passes against a running QEMU instance.
- A Robot Framework test using `qemu_keywords.robot` passes the same scenario.

**Note on timeouts**: In standalone mode QEMU runs at ~real-time so wall-clock timeouts
work. In `slaved-icount` mode QEMU runs at ~15% speed and wall-clock timeouts become
incorrect. Virtual-time-aware timeouts (polling `query-cpus-fast` for vtime) are deferred
to Phase 7 when slaved modes are active.

### Tasks
- [ ] **4.1** Write `tools/testing/qmp_bridge.py`:
  - Async wrapper around `qemu.qmp` library
  - `connect(socket_path)`, `execute(cmd, args)`, `wait_for_event(event_name)`
  - UART monitoring: connect to QEMU chardev socket, non-blocking readline
  - Use `query-cpus-fast` (NOT deprecated `query-cpus`)
  - Expose `get_virtual_time_ns()` using accumulated clock advance state (for Phase 7)

- [ ] **4.1b** Write `tools/testing/test_qmp.py` (pytest):
  - pytest fixtures: `qemu_process`, `qmp_bridge`, `uart_socket`
  - Test functions mirror the Robot keyword set (same coverage, better debuggability)
  - This is the **primary** test implementation; Robot keywords are the compatibility layer

  **Critical isolation constraint — standalone mode only for Phase 4 tests:**
  Phase 4 tests run without MuJoCo. If QEMU is launched with `-device zenoh-clock`
  (the Phase 7 plugin), it will block at the first TCG TB boundary waiting for a
  TimeAuthority clock advance that never arrives, hanging the test indefinitely.

  The `qemu_process` fixture **must** start QEMU without the zenoh-clock device. Enforce
  this explicitly — do not rely on the device being absent from the build. Either:
  - Use a QEMU command that never includes `-device zenoh-clock`, or
  - Confirm the fixture arguments list and assert the string is absent before launching.

  Phase 7 integration tests (task 7.4) are a separate pytest suite that starts a mock
  TimeAuthority alongside QEMU. They must not run in regular CI without the full
  FirmwareStudio stack.

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
- [ ] **5.1** Implement Path A prerequisite: write `hw/misc/mmio-socket-bridge.c` — a
      custom QOM `SysBusDevice` that registers a `MemoryRegion`, intercepts
      `MemoryRegionOps` read/write via a Unix socket request-response protocol, and
      forwards them to an external C++ SystemC adapter. QEMU does NOT natively serialize
      MMIO to chardev sockets — this device is required before Path A is usable.
      Then write `tools/systemc_adapter/` — C++ shim translating those socket messages
      to SystemC TLM-2.0 `b_transport` calls. Validate with a simple register-file model.
      *(No Python daemons. No Verilated models needed to start.)*
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

**Background**: FirmwareStudio (upstream repository) already has a working prototype:
- `physics/time_authority/` — Python `TimeAuthority` class running in MuJoCo container
- `cyber/patches/0001-add-libqemu-clocksock.patch` — QEMU patch that exposes a Unix socket
- `cyber/src/node_agent.py` — bridges Zenoh ↔ QEMU Unix socket
- `cyber/src/shm_bridge.py` — bridges IVSHMEM MMIO ↔ Zenoh for sensor/actuator I/O

qenode's job in Phase 7: replace the prototype with production-quality implementations.

### Three Clock Modes

| Mode | QEMU flag | Performance | Use when |
|---|---|---|---|
| `standalone` | (none) | 100% | Development, CI without physics |
| `slaved-suspend` | (none — native plugin) | ~95% | FirmwareStudio default — TB-boundary halt via hw/zenoh/ |
| `slaved-icount` | `-icount shift=0,align=off,sleep=off` | ~15–20% | Sub-quantum timing needed (PWM, hardware timers) |

Inspired by Qualcomm qbox's `libgssync` cooperative-suspend pattern:
`slaved-suspend` hooks into the TCG loop at translation-block boundaries — no icount
penalty, full TCG speed within each step. The ±1 TB jitter is irrelevant for control loops.
**This is implemented as a native C module, not via external Python QMP commands.**

### Design: External Time Master Protocol

```
MuJoCo (mj_step)
    → TimeAuthority.step(quantum_ns)
        → Zenoh: GET sim/clock/advance/{node_id}  payload=(delta_ns, mujoco_time_ns)
            → hw/zenoh/zenoh-clock.c (native C plugin inside QEMU)
                → Blocks QEMU TCG loop at translation-block boundary
                → (slaved-icount only) qemu_icount_bias += delta_ns
                → QEMU runs delta_ns ns of virtual time, then blocks again
                → Replies: ClockReady{vtime_ns, n_frames}
            ← Zenoh reply: ClockReady
        ← TimeAuthority routes Ethernet frames via sim/eth/frame topics
```

**slaved-icount** requires: `-icount shift=0,align=off,sleep=off`
**slaved-suspend** requires no extra flags — the plugin handles halting via TCG hooks.

### Performance of the External Clock Approach

The stepping protocol adds overhead per quantum (typically 1–10 ms of sim time per step):
- Zenoh round-trip (same machine, Unix socket backend): ~10–50 µs
- TCG hook block/resume: ~1 µs
- For a 1 kHz physics loop (1 ms quantums): stepping overhead is < 5% of wall time

**slaved-suspend (default)**: QEMU runs at full TCG speed within each quantum. The only
penalty is the ~50 µs Zenoh round-trip at quantum boundaries. For a 1 kHz loop that is
~5% overhead — negligible.

**slaved-icount (sub-quantum precision only)**: icount mode disables TCG translation block
chaining, reducing raw instruction throughput by ~5–10×. A Cortex-A15 in QEMU delivers
~100–200 MIPS without icount; ~20–40 MIPS with it. This is still sufficient for 1 kHz
PID loops (~10 000 instructions/iteration) with 2–4× headroom. At 10 kHz the margin
tightens; prefer slaved-suspend if the firmware does not need sub-quantum timer precision.

### Tasks
- [ ] **7.1** Write `hw/zenoh/zenoh-clock.c` — native QOM device (SysBusDevice):
  - Links `zenoh-c` (added to QEMU Meson as a `dependency()`)
  - Declares a Zenoh queryable on `sim/clock/advance/{node_id}` at `realize` time
  - Assigns its blocking routine to the exposed `qenode_tcg_quantum_hook` function pointer (installed by `apply_zenoh_hook.py`). This is required because QEMU exports no dynamic APIs for QOM modules to hook the internal `cpu_exec` loop.
  - Compiles as `hw-qenode-zenoh.so` via the existing Meson module system

  **Critical implementation constraint — BQL (Big QEMU Lock):**
  The TCG vCPU thread holds the BQL while executing translated code. Blocking on a Zenoh
  reply without releasing it will deadlock the entire process: the main event loop thread
  (QMP socket, GDB stub, I/O) cannot acquire the BQL to service requests.

  Correct sequence for `slaved-suspend`:
  1. Call `cpu_exit(cpu)` to set the TB-exit flag; the TCG loop exits cleanly at the
     current TB boundary without blocking mid-translation.
  2. In the outer vCPU loop (between TB dispatch iterations), before re-entering the
     next TB, our hook fires.
  3. Call `bql_unlock()` to release the lock.
  4. Block on the Zenoh queryable reply from TimeAuthority.
  5. Call `bql_lock()` to re-acquire before returning to the TCG loop.

  This is the standard QEMU pattern for vCPU-thread blocking operations (same as
  `qemu_mutex_unlock_iothread()` / `qemu_mutex_lock_iothread()` used in device models).
  Never skip step 3 or omit step 5 — partial locking causes silent data corruption.

  For `slaved-icount`: same BQL sandwich applies; additionally call
  `cpu_icount_advance(cpu, delta_ns)` (or directly set `timers_state.qemu_icount_bias`)
  while holding the BQL after step 5.

- [ ] **7.2** Write `hw/zenoh/zenoh-netdev.c` — custom `-netdev` backend:
  - Implements `NetClientInfo` with `receive` (host→guest) and `can_receive`
  - TX path: serializes frame + current virtual timestamp → Zenoh publish to
    `sim/eth/frame/{node_id}/tx`
  - Registered as `-netdev zenoh,id=...,node=...,router=...`

  **Critical implementation constraint — virtual-time frame injection:**
  QEMU does not poll internal state for timestamp thresholds. A netdev backend cannot
  "wait" for virtual time to reach an arbitrary value and then inject a frame
  spontaneously. Frame injection must be driven by QEMU's timer subsystem.

  Correct RX implementation:
  1. Zenoh subscription callback (fires in Zenoh's thread): parse incoming frame,
     extract `delivery_vtime`, insert into a **min-heap priority queue** keyed by
     `delivery_vtime`.
  2. After inserting, call `timer_mod(rx_timer, delivery_vtime_of_earliest_frame)` to
     arm (or re-arm) the `QEMUTimer`.
  3. Allocate the timer with `timer_new_ns(QEMU_CLOCK_VIRTUAL, rx_timer_cb, netdev)` at
     init time. `QEMU_CLOCK_VIRTUAL` ensures the timer fires relative to QEMU's virtual
     clock — in slaved-icount mode this is the icount clock; in slaved-suspend mode it
     fires when QEMU resumes and virtual time catches up.
  4. In `rx_timer_cb`: drain all frames from the priority queue whose
     `delivery_vtime <= qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL)`, calling
     `qemu_send_packet(nc, frame_data, frame_len)` for each. Re-arm the timer for the
     next earliest frame if the queue is non-empty.
  5. The Zenoh callback runs in a foreign thread; the timer callback runs in the QEMU
     main loop thread. The priority queue must be protected by a `QemuMutex`.

- [ ] **7.3** Delete `tools/node_agent/` — superseded by hw/zenoh/

- [ ] **7.4** Integration test: boot minimal firmware, step 1000 × 1 ms, assert
  firmware timestamps are deterministic across two identical runs.

- [ ] **7.5** Replace FirmwareStudio's `cyber/` with a dependency on qenode:
  - `worlds/*.yml` Docker Compose files reference qenode's patched QEMU image
  - Remove `cyber/src/node_agent.py` — replaced by `hw/zenoh/` native plugin

---

## Phase 6 — Multi-Node Coordination ⬜ (Future)

**Goal**: Deterministic multi-node network simulation replacing Renode's `WirelessMedium`,
implemented as a native Zenoh netdev backend inside QEMU.

**Important**: `-netdev socket,mcast=...` + icount does NOT give determinism. UDP multicast
delivery is scheduled by the host kernel regardless of QEMU's virtual clock. icount makes
QEMU's internal instruction counting deterministic but cannot control when the kernel
delivers a UDP datagram to QEMU's receive path.

**Design**:
- `hw/zenoh/zenoh-netdev.c` — custom QEMU `-netdev` backend (no Python coordinator):
  - TX: guest NIC raises DMA → netdev publishes frame to `sim/eth/frame/{node_id}/tx`
    with embedded virtual timestamp
  - RX: netdev subscribes `sim/eth/frame/{node_id}/rx`; incoming frames are buffered
    and injected into the guest NIC only when virtual time reaches the stamped arrival time
- A lightweight C/Rust coordinator process (not Python) subscribes all TX topics,
  applies the attenuation/distance model, and republishes to RX topics with adjusted
  virtual timestamps
- Determinism comes from virtual-timestamp ordering, not from UDP delivery timing

---

## Risks and Open Questions

| # | Risk | Mitigation |
|---|------|-----------|
| R1 | arm-generic-fdt patchew series may not apply cleanly to v10.2.92 HEAD | Pin to the exact commit the patchew was submitted against; cherry-pick conflicts manually |
| R2 | Native module approach fails on some macOS builds | Omit `--enable-plugins` on Darwin natively to bypass GLib symbol conflict |
| R3 | macOS `.so` loading is broken with `--enable-plugins` | Enforce Linux-only dev environment in CI |
| R4 | Native Zenoh plugin (`hw/zenoh/`) adds `zenoh-c` as a QEMU Meson dependency | Pin zenoh-c version; vendor as Meson `subproject()` to avoid system-library conflicts |
| R5 | Renode .repl parser has undocumented edge cases | Use Renode source (`third_party/renode`) as ground truth; diff parser output against Renode's own AST |
| R6 | `arm-generic-fdt` v3 patch series may have changed between patchew submission and merger | Track patchew thread; re-fetch if a v4 series is posted |
| R7 | icount mode reduces firmware execution speed ~5–10× | Acceptable for control loops ≤10 kHz; profile with `perf` if needed |
| R8 | FirmwareStudio `libqemu` patch uses placeholder git hashes (aaaa/bbbb) and may not apply | Must be manually rewritten with real context lines against QEMU 11.0.0-rc2 |
| R9 | `zenoh-c` adds a native dependency to QEMU's Meson build | Pin zenoh-c version in Dockerfile; vendor or wrap as a Meson `subproject()` to avoid system-library conflicts |
| R10 | TCG cooperative-halt hooks may conflict with future QEMU upstream refactors | Keep hook surface minimal; track QEMU `accel/tcg/` API changes on each upstream bump |

---

## Deferred / Won't Do (Phase 1-4 scope)

- Windows support (module loading fundamentally broken on Windows with current QEMU)
- RISC-V until ARM is validated
- RESD (Renode Sensor Data) format injection
- Antigravity IDE / agent_memory.json / mcp_servers.json (not project artifacts)
- `query-cpus` (deprecated — use `query-cpus-fast` only)

## Permanently Rejected Approaches

These were evaluated and will not be revisited unless the stated reasons change.
See `docs/ARCHITECTURE.md §10` for full rationale.

| Approach | Reason rejected |
|---|---|
| Python node_agent for clock sync (QMP stop/cont) | Asynchronous QMP dispatch gives indeterminate virtual-time halt boundaries; OS thread jitter breaks causal consistency (ADR-001) |
| Python vhost-user daemons for peripherals | ~1–5 µs/call IPC latency; catastrophic at >100 kHz access rates; violates No-Python-in-Loop (ADR-003) |
| `-netdev socket,mcast=...` for multi-node networking | UDP delivery is OS-scheduled, not gated on virtual time; icount mode does not fix this (ADR-002) |
| Robot Framework as primary test framework | Wall-clock timeouts break in slaved-icount mode; pytest + qemu.qmp is more debuggable and QEMU-upstream-aligned (ADR-004) |
| QEMU chardev sockets as native MMIO proxies | QEMU chardev is a byte stream for UARTs only; does not serialize MemoryRegionOps; custom bridge device required (ADR-005) |
| Full qbox/MINRES SystemC-as-kernel embedding | SystemC dependency is overkill; Zenoh provides equivalent IPC; TCG hook pattern from qbox adopted without the SystemC layer (ADR-001, §8) |
