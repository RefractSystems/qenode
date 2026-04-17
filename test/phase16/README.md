# Phase 16: Performance & Determinism CI

This directory contains a benchmarking suite for `virtmcu`.

## Key Metrics Measured
- **IPS (Instructions Per Second)**: Raw emulation speed in MIPS.
- **Determinism**: Verification that instruction counts are identical across multiple slaved-icount runs.
- **Co-simulation Latency**: Round-trip overhead for Zenoh-based clock synchronization.

## Benchmarking Results (Typical)
| Mode | Speed (MIPS) | Deterministic? | Latency (mean RT) |
|---|---|---|---|
| `standalone` | ~2700 (est) | No | N/A |
| `slaved-icount` | ~1100 | **Yes** | ~1–3 ms |

**IPS methodology**: `slaved-icount` runs with `-icount shift=0` (1 instruction = 1 ns virtual time), so
`current_vtime_ns` at EXIT is a direct instruction-count proxy. MIPS = `exit_vtime_ns / wall_time / 1e6`.

**Determinism**: two independent `slaved-icount` runs must produce identical `exit_vtime_ns` values.

**Latency**: 1 ms quantums are used so that ~40 round-trip samples are collected per run; p95/p99 are reported.

## Usage
Run the automated test:
```bash
./smoke_test.sh
```

Or run the python script directly:
```bash
python3 bench.py
```
