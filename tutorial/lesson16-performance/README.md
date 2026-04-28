# Tutorial Lesson 16: Profiling and Benchmarking virtmcu

**Audience**: CS graduate students and engineers familiar with computer architecture concepts but new to deterministic simulation.

**Prerequisites**: Tutorial Lessons 1–7 (QEMU build, FDT boot, clock synchronization).

---

## 1. Why Performance Benchmarking Matters in Deterministic Simulation

virtmcu provides **deterministic virtual-time execution** — every firmware run with the same inputs produces byte-perfect identical output. This property is only useful if the underlying emulation is fast enough to run tests in reasonable wall-clock time.

Two competing goals create a natural tension:

| Goal | Effect on performance |
|---|---|
| Determinism | Requires blocking the vCPU at quantum boundaries → overhead |
| Speed | Wants the vCPU to run uninterrupted |

Lesson 16 introduces systematic benchmarking so you can measure the **actual overhead** of the Zenoh-based clock synchronization and detect performance regressions automatically in CI.

---

## 2. Key Metrics

### 2.1 Instructions Per Second (IPS / MIPS)

MIPS = millions of emulated ARM instructions executed per second of wall-clock time.

virtmcu measures MIPS in two modes:

| Mode | How IPS is measured | Expected baseline |
|---|---|---|
| `standalone` | Firmware reads CNTVCT before and after a tight MULS loop; difference ÷ wall time | ≥ 80 MIPS |
| `slaved-icount` | `exit_vtime_ns` from Zenoh clock reply (icount shift=0 → 1 ns = 1 instruction) ÷ wall time | ≥ 15 MIPS |

**Why `slaved-icount` is slower**: every quantum boundary (default 10 ms virtual time = 10 million instructions) requires a Zenoh GET round-trip. The vCPU blocks until the TimeAuthority responds. The Zenoh RTT plus QEMU TCG re-entry overhead reduces throughput.

### 2.2 Co-simulation Latency

**Latency** = wall-clock time from the moment virtmcu sends a Zenoh clock-advance GET to the moment the reply arrives.

| Threshold | Meaning |
|---|---|
| P50 ≤ 200 µs | Typical loopback Zenoh round-trip |
| P99 ≤ 1 ms | Maximum acceptable tail latency |
| Fail if P50 > 500 µs | System is overloaded or Zenoh router is slow |
| Fail if P99 > 2 ms | Unacceptable jitter |

### 2.3 Determinism

Two independent `slaved-icount` runs must produce **identical** `exit_vtime_ns` values (exact instruction counts). Any difference means a non-deterministic input entered the simulation.

---

## 3. Benchmark Architecture

```
  ┌───────────────────────────────────────────────────────────────┐
  │  bench.py (TimeAuthority mock + measurement harness)           │
  │                                                                │
  │   1. Compiles minimal.dtb from src/minimal.dts                 │
  │   2. Starts ephemeral Zenoh router (prevents multicast noise)  │
  │   3. Launches QEMU with bench.elf                              │
  │   4. For slaved-icount: sends Zenoh GETs, records RTT         │
  │   5. For standalone: measures wall time until EXIT signal      │
  │   6. Emits JSON results and checks thresholds                  │
  └───────────────────────────────────────────────────────────────┘
```

### The Benchmark Firmware (`bench.c`)

The firmware executes a tight multiply loop, then reports the ARM Generic Timer counter:

```c
// Exit condition: enough MULS to produce ~1M instructions
for (uint32_t i = 0; i < LOOP_COUNT; i++) {
    result *= (i | 1);  // non-zero multiply to prevent optimization
}
uint64_t cycles = read_cntvct();
printf("CYCLES: %llx\n", cycles);
printf("EXIT\n");
```

**Why use `CNTVCT`?** The ARM Generic Timer counter increments at a fixed frequency (CNTFRQ, typically 1 GHz in QEMU). It provides a consistent elapsed-time measurement independent of the MIPS calculation.

**Why use `exit_vtime_ns` for MIPS in icount mode?** With `-icount shift=0`, QEMU advances virtual time by exactly 1 ns per instruction. So `exit_vtime_ns` after the firmware prints `EXIT` equals the number of instructions executed. This is more accurate than CNTVCT, which reflects the ARM counter frequency, not instruction count.

---

## 4. Running the Benchmark

### 4.1 Full benchmark (all modes)

```bash
cd tutorial/lesson16-performance
make
python3 src/bench.py
```

Expected output:

```
--- [standalone] ---
  [QEMU/standalone/stdout] CNTFRQ: 3B9ACA00
  [QEMU/standalone/stdout] CYCLES: 9F5E100
  [QEMU/standalone/stdout] EXIT
  wall  : 4.231 s
  cycles: 167,116,032
--- [slaved-icount] ---
  ...
  rtt   : min=0.80 mean=1.20 p95=2.10 p99=3.40 max=5.60 ms  (n=42)
--- [slaved-icount-2] ---
  ...

=== Performance Summary ===
Determinism          : PASSED  (167,116,032 vs 167,116,032 cycles)
slaved-icount MIPS   : 1142.3
{"mode": "slaved-icount", "mips": 1142.3}
standalone MIPS (est): 2891.7
{"mode": "standalone", "mips": 2891.7}
Co-sim latency       : min=0.80 mean=1.20 p95=2.10 p99=3.40 max=5.60 ms  (n=42)
{"p50_us": 1050.0, "p99_us": 3400.0, "stalls": 0}
=== Lesson 16 PASSED ===
```

### 4.2 Smoke test (CI entry point)

```bash
bash src/smoke_test.sh
```

This rebuilds the firmware, runs `bench.py`, and fails if any threshold is violated.

---

## 5. Interpreting Results

### 5.1 MIPS is lower than expected

**Common causes**:
1. **Zenoh multicast scouting**: if you run `bench.py` without an explicit router URL, Zenoh wastes ~200 ms discovering peers. The benchmark always passes a router URL to avoid this.
2. **Shared CI runner**: absolute MIPS thresholds assume a dedicated host. On a shared runner, trend-based comparison is preferred.
3. **Debug QEMU build**: QEMU built with `--enable-debug` runs 2–4× slower. Use `--disable-debug` for performance measurements.

### 5.2 P99 latency is high

**Common causes**:
1. **GC pause in Zenoh router**: restart the router if P99 spikes on the first few quanta but settles afterward.
2. **CPU frequency scaling**: CI runners may throttle under thermal load. Check with `cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor`.
3. **QEMU stall timeout too short**: the stall timeout (default 5 s) may be triggering. Check error_code in Zenoh replies.

### 5.3 Determinism check fails

A non-zero cycle delta between two `slaved-icount` runs means **non-deterministic input** entered the simulation:
- Check for `gettimeofday` calls in firmware (forbidden — use virtual timer).
- Check for `rand()` without a fixed seed.
- Check that the Zenoh router is the same between runs (different routers = different multicast topology = different startup latency).

---

## 6. Jitter Injection Test

The jitter test proves that virtmcu's virtual-time gating correctly neutralizes host network jitter. It works by:

1. Starting a `jitter_proxy.py` that intercepts Zenoh clock messages and adds a random ±200 µs delay before forwarding.
2. Running 5 independent `slaved-icount` benchmark runs through the proxy.
3. Asserting all 5 `exit_vtime_ns` values are identical.

```bash
bash src/jitter_test.sh
```

**Why this matters**: the proxy simulates a noisy CI environment (shared network, GC pauses, OS scheduler latency). If the virtual-time gating is correct, the jitter is absorbed entirely in wall-clock time — the firmware sees only a perfectly regular virtual clock.

---

## 7. Hands-on Exercise

**Goal**: observe the MIPS overhead of adding a second slaved peripheral.

1. Run the baseline benchmark: `python3 src/bench.py`
2. Modify `src/bench.py` to add `-device telemetry,node=0,router=<router>` to the QEMU command for `slaved-icount` mode (note: ensure `telemetry` is available in your build).
3. Re-run and compare MIPS. You should see a 1–5% overhead from the telemetry IRQ hook.

**Questions to consider**:
- Does adding telemetry affect determinism?
- Does the overhead grow linearly with IRQ rate?
- At what telemetry event rate does the P99 latency threshold get exceeded?

---

## Summary

| Metric | Target | Fail threshold |
|---|---|---|
| standalone MIPS | ≥ 80 | < 60 |
| slaved-icount MIPS | ≥ 15 | < 10 |
| P50 RTT | ≤ 200 µs | > 500 µs |
| P99 RTT | ≤ 1 ms | > 2 ms |
| Stalls | 0 | any |
| Determinism delta | 0 cycles | any |
