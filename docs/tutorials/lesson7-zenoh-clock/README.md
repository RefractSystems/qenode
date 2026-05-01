# Lesson 7 — Zenoh Clock: Deterministic Co-simulation Time Synchronization

This lesson explains the `virtmcu-clock` QOM device (`hw/rust/clock`), which
makes QEMU run as a **time slave** to an external physics simulation (MuJoCo/FirmwareStudio).

---

## Background

In a hardware-in-the-loop or digital twin setup, virtual time must stay causally
consistent with physical simulation time.  The external `TimeAuthority` (running in
the MuJoCo container) is the time master.  QEMU must:

1. **Not run ahead** of the physics simulation.
2. **Advance by exactly** the number of nanoseconds the physics step computed.
3. **Block** at the end of each quantum until the next step arrives.

The `virtmcu-clock` plugin implements this contract without any Python in the simulation
loop.  It links `zenoh-c` directly into QEMU and hooks into the TCG execution loop.

---

## Architecture

```
TimeAuthority (Python, MuJoCo container)
      │  Zenoh: sim/clock/advance/0  (query with delta_ns payload)
      ▼
hw/rust/clock  ──  on_query()
      │  wakes vCPU, waits for quantum completion
      ▼
zclock_quantum_hook()  (called at every TCG translation-block boundary)
      │  blocks vCPU, reads virtual clock, signals on_query with vtime_ns
      ▼
TimeAuthority receives reply: current_vtime_ns
```

---

## Two Modes

### suspend mode (default)

```
-device virtmcu-clock,mode=suspend,node=0
```

TCG runs at **full host speed** between quanta. At each TB boundary the hook
checks whether the virtual timer has fired. The sequence of operations is:

1. The `TimeAuthority` sends a `delta_ns` via Zenoh. `on_query` deposits this delta, wakes the hook, and blocks waiting for the quantum to complete.
2. The hook wakes, re-acquires BQL, arms the timer for `now + delta_ns`, and returns.
3. The vCPU continues executing at full speed until the timer fires.
4. The timer callback forces the vCPU to exit and re-enter the hook.
5. The hook snapshots the new virtual time, signals `on_query`, releases BQL, and blocks waiting for the next delta.
6. `on_query` wakes, reads the new `vtime_ns`, and sends the Zenoh reply back to the `TimeAuthority`.

Use suspend mode unless firmware measures intervals shorter than one physics quantum.
It gives ~95% of unconstrained TCG speed.

### icount mode

```
-icount shift=0,align=off,sleep=off
-device virtmcu-clock,mode=icount,node=0
```

QEMU's instruction counter drives virtual time. `on_query` performs the exact same handshake as suspend mode to guarantee strict causal consistency. The `qemu_icount_bias` is advanced in Step 8 of the hook (after the handshake), not directly in `on_query`. The TimeAuthority waits for the hook to complete the quantum before receiving a reply. Use this when firmware timestamps need sub-quantum precision.

---

## Lock Ordering (suspend mode)

The most important invariant in this device — violating it causes deadlock:

| Thread | Locks acquired (in order) |
|--------|--------------------------|
| vCPU (hook) | BQL → `s->mutex` |
| Zenoh (`on_query`) | `s->mutex` **only** — **never** calls `bql_lock()` |

The hook must release BQL before waiting on a condition variable.  `on_query` must
not acquire BQL in the suspend path.

---

## Wire Protocol

### Query payload (`ClockAdvancePayload`, 24 bytes, little-endian)

| Offset | Size | Field | Description |
|--------|------|-------|-------------|
| 0 | 8 | `delta_ns` | Nanoseconds to advance virtual time |
| 8 | 8 | `mujoco_time_ns` | MuJoCo wall time (informational) |
| 16 | 8 | `quantum_number` | Monotonically increasing quantum ID |

### Reply payload (`ClockReadyPayload`, 24 bytes, little-endian)

| Offset | Size | Field | Description |
|--------|------|-------|-------------|
| 0 | 8 | `current_vtime_ns` | Virtual clock value after the quantum |
| 8 | 4 | `n_frames` | Reserved, always 0 |
| 12 | 4 | `error_code` | 0=OK, 1=STALL, 2=ZENOH_ERROR |
| 16 | 8 | `quantum_number` | Echoed quantum ID for verification |

---

## Running the Example

### Prerequisites

- QEMU built with the clock module (see `scripts/setup-qemu.sh`).
- `zenoh-c` library in `third_party/zenoh-c/`.
- `eclipse-zenoh` Python package (`uv sync` or `uv pip install eclipse-zenoh`).
- `arm-none-eabi-gcc` and `dtc` on PATH.

### Quick test (both modes)

```bash
bash tests/fixtures/guest_apps/clock_suspend/smoke_test.sh
```

### Manual walkthrough

```bash
# Build a trivial firmware blob
cat > /tmp/spin.S <<'EOF'
.global _start
_start: b _start
EOF
arm-none-eabi-gcc -mcpu=cortex-a15 -nostdlib \
    -Wl,--section-start=.text=0x40000000 \
    /tmp/spin.S -o /tmp/spin.elf

# Generate a minimal DTB (see tests/fixtures/guest_apps/clock_suspend/smoke_test.sh for full DTS)
# ...

# Start QEMU in suspend mode
scripts/run.sh \
    --dtb tests/fixtures/guest_apps/boot_arm/minimal.dtb \
    -kernel /tmp/spin.elf \
    -device virtmcu-clock,mode=suspend,node=0 \
    -nographic -monitor none &

# Send a clock-advance query from Python
python3 - <<'PY'
import zenoh, vproto, time, sys
s = zenoh.open(zenoh.Config())
time.sleep(1)  # wait for QEMU to register queryable
payload = vproto.ClockAdvanceReq(5_000_000, 0, 0).pack()  # advance 5 ms, q=0
for reply in s.get("sim/clock/advance/0", payload=payload, timeout=5.0):
    resp = vproto.ClockReadyResp.unpack(reply.ok.payload.to_bytes())
    if resp.error_code == 0:
        sys.stdout.write(f"Virtual time after quantum: {resp.current_vtime_ns} ns\n")
    else:
        sys.stderr.write(f"Error: {resp.error_code}\n")
s.close()
PY
```

Expected output:
```
Virtual time after quantum: 5000000 ns
```

---

## Device Properties

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `mode` | string | `suspend` | Clock mode: `suspend` or `icount` |
| `node` | uint32 | `0` | Node ID — appended to the Zenoh topic |
| `router` | string | (none) | Zenoh router endpoint (future use) |

---

## State Machine Diagram (suspend mode)

```
QEMU realize()
    │ needs_quantum=true
    ▼
vCPU starts ──► hook() [needs_quantum=true]
                   │ capture vtime_ns, set quantum_done
                   │ release BQL, wait on vcpu_cond (blocks until next query)
                   ▼
    on_query() ◄───┘ (Zenoh: TimeAuthority sends delta_ns)
        │ store delta_ns, set quantum_ready, signal vcpu_cond
        │ wait on query_cond
        ▼
    hook() wakes
        │ re-acquire BQL, timer_mod(now + delta_ns)
        │ return → vCPU runs
        ▼
    timer fires ──► timer_cb() sets needs_quantum, cpu_exit()
        │
    hook() [needs_quantum=true]
        │ capture vtime_ns, set quantum_done, signal query_cond
        │ release BQL, wait on vcpu_cond (blocks until next query)
        ▼
    on_query() wakes
        │ read vtime_ns
        │ send Zenoh reply (current_vtime_ns) ──► TimeAuthority
```

---

## Relationship to FirmwareStudio

`clock` is the QEMU-side half of the FirmwareStudio time synchronization
protocol.  The other half is the `TimeAuthority` in `cyber/src/time_authority.py`
(MuJoCo container).  Together they guarantee that firmware never runs ahead of the
physics simulation, giving causally consistent sensor readings and actuator responses.

See `docs/architecture/01-system-overview.md` §"FirmwareStudio Integration" for the full picture.
