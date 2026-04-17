# virtmcu Timing Model — Reference

This document is the canonical reference for timing behavior in virtmcu.
For the tutorial-style explanation with sequence diagrams, see
[TIME_MANAGEMENT_DESIGN.md](TIME_MANAGEMENT_DESIGN.md).

---

## Clock Modes

| Mode | `-device zenoh-clock` | Additional flags | Effective throughput |
|---|---|---|---|
| `standalone` | No | — | 100% of host TCG speed |
| `slaved-suspend` | Yes, `mode` omitted or `mode=suspend` | — | ~95% — one Zenoh round-trip overhead per quantum |
| `slaved-icount` | Yes, `mode=icount` | `-icount shift=0,align=off,sleep=off` | ~15–20% — TB chaining disabled |

**Choosing a mode:**

```
Does firmware measure intervals shorter than one physics quantum?
    No  → slaved-suspend   (default; ±dt jitter is invisible to control loops)
    Yes → slaved-icount    (required for PWM generation, µs DMA, tick-counting)
```

---

## Wire Protocol

### Request — TimeAuthority → QEMU

Topic: `sim/clock/advance/{node_id}`

```c
typedef struct __attribute__((packed)) {
    uint64_t delta_ns;        /* quantum size in virtual nanoseconds */
    uint64_t mujoco_time_ns;  /* current physics world time          */
} ClockAdvancePayload;        /* 16 bytes total                       */
```

### Reply — QEMU → TimeAuthority

```c
typedef struct __attribute__((packed)) {
    uint64_t current_vtime_ns; /* QEMU_CLOCK_VIRTUAL after the quantum; 0 on error */
    uint32_t n_frames;         /* pending Zenoh Ethernet frames (informational)     */
    uint32_t error_code;       /* see table below                                   */
} ClockReadyPayload;           /* 16 bytes total                                    */
```

### Error Codes

| Code | Name | Meaning | Likely cause |
|---|---|---|---|
| `0` | `OK` | Quantum completed; `current_vtime_ns` is valid | — |
| `1` | `STALL` | QEMU did not reach TB boundary within the stall timeout (default 5 s; set via `stall-timeout=<ms>` device property) | Firmware crash, infinite loop, or host overload |
| `2` | `ZENOH_ERROR` | Transport-level failure before or during reply | Malformed payload, session drop, router unreachable |

---

## Virtual Time Advancement Rules

| CPU state | `slaved-suspend` | `slaved-icount` |
|---|---|---|
| Executing instructions | Advances with TCG at full speed | Advances at exactly 1 ns per instruction (`shift=0`) |
| `WFI` with pending timers | Skips forward to nearest `QEMUTimer` deadline | Same — time warps to next timer expiry |
| `WFI` with no timers | Frozen until external IRQ | Frozen until external IRQ |
| Blocked in `mmio-socket-bridge` | Frozen (vCPU not executing) | Frozen (no instructions counted) |
| Blocked at quantum boundary | Frozen (waiting for TimeAuthority reply) | Frozen |

**Key invariant**: Host-side latency (socket round-trip, Python adapter processing time) has
zero effect on guest virtual time. Virtual time is driven entirely by instruction count or
timer expiry, never by wall-clock.

---

## Quantum Boundary Sequence

```
TCG hook fires (end of translation block)
  │
  ├─ needs_quantum == false → return immediately (keep executing)
  │
  └─ needs_quantum == true
       │
       ├─ Set quantum_done = true
       ├─ Signal query_cond  (wakes on_query thread)
       ├─ bql_unlock()
       ├─ Wait on vcpu_cond  ← QEMU paused here; BQL released
       │     (on_query sends Zenoh reply, reads next delta_ns, signals vcpu_cond)
       ├─ bql_lock()
       ├─ [icount only] advance qemu_icount_bias by delta_ns
       ├─ timer_mod(quantum_timer, now + delta_ns)
       └─ return to TCG
```

The BQL is released before the wait and re-acquired after, keeping the QEMU event
loop (QMP, GDB stub, chardev I/O) alive during the pause.

---

## BQL Rules

| Action | BQL state required |
|---|---|
| Zenoh `GET` / `z_query_reply` | **Released** — Zenoh callbacks run on a background thread |
| `timers_state.qemu_icount_bias` update | **Held** |
| `timer_mod` | **Held** |
| `cpu_exit()` | Either (safe to call from any thread) |

Violating these rules causes a deadlock. The main symptom is QEMU freezing with the
QMP monitor and UART both unresponsive.

---

## Performance Notes

- Zenoh round-trip (localhost router): ~10–50 µs → `slaved-suspend` overhead is ~1–5%
  at a 1 ms quantum.
- `slaved-icount` at `shift=0` counts every instruction individually, disabling TB
  chaining and most TCG optimizations. Expect 300–600 MIPS to drop to 20–40 MIPS.
- KVM/hvf acceleration is incompatible with all slaved modes (TCG hooks are unreachable
  under hardware virtualization) and with all Cortex-M targets.

---

## Related Documents

- [TIME_MANAGEMENT_DESIGN.md](TIME_MANAGEMENT_DESIGN.md) — tutorial, sequence diagrams, rationale
- [ARCHITECTURE.md](ARCHITECTURE.md) — system context, Five Pillars, prior art
- [ADR-001](ARCHITECTURE.md#adr-001-three-clock-modes-standalone--slaved-suspend--slaved-icount) — rationale for three modes
- `hw/zenoh/zenoh-clock.c` — implementation
- `patches/apply_zenoh_hook.py` — TCG hook injection
